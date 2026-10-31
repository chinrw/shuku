// Service Worker：缓存网页端自己的文件和最近读过的页面，离线时能打开书库和读过的书。
// 只在 HTTPS（例如 tailscale serve）或 localhost 下生效，局域网 HTTP 下浏览器不提供 Service Worker。
import { self } from '$app/service-worker';
import { immutable, assets } from '$app/manifest';
import { version } from '$app/env';
import { BoundedCache } from './bounded.ts';
import { route } from './routes.ts';

const SHELL = `shell-${version}`;
const SHELL_FILES = ['/index.html', ...[...immutable, ...assets].map(({ path }) => `/${path}`)];

const MiB = 1024 * 1024;

// 浏览器给每个网站的空间差别很大（iOS 上可能只有几百 MB 到几 GB），所以原图最多占配额的
// 四分之一，再封顶 1 GiB，按每页 1 到 3 MB 算大约能留十几本。
let quota: Promise<number> | null = null;
function share(fraction: number, max: number) {
	return async () => {
		// Safari 16 的 Worker 里有 storage 但没有 estimate。
		quota ??= (self.navigator.storage?.estimate?.() ?? Promise.resolve({ quota: undefined })).then(
			(estimate) => estimate.quota ?? Infinity,
			() => Infinity
		);
		return Math.min(max, (await quota) * fraction);
	};
}

// 名字带版本号是为了以后改格式时能整个丢掉；网页端升级时不清这几个缓存。
const pages = new BoundedCache('pages-v1', share(0.25, 1024 * MiB));
const thumbnails = new BoundedCache('thumbnails-v1', share(0.05, 200 * MiB));
const api = new BoundedCache('api-v1', share(0.02, 30 * MiB));
const KEPT = new Set([SHELL, 'pages-v1', 'thumbnails-v1', 'api-v1']);

self.addEventListener('install', (event) => {
	event.waitUntil(
		(async () => {
			const cache = await caches.open(SHELL);
			// 绕过 HTTP 缓存，免得把旧的 index.html 当成新版本存进去。
			await cache.addAll(SHELL_FILES.map((path) => new Request(path, { cache: 'reload' })));
			await self.skipWaiting();
		})()
	);
});

self.addEventListener('activate', (event) => {
	event.waitUntil(
		(async () => {
			for (const name of await caches.keys()) if (!KEPT.has(name)) await caches.delete(name);
			await self.clients.claim();
		})()
	);
});

self.addEventListener('fetch', (event) => {
	const kind = route(event.request, self.location.origin);
	switch (kind) {
		case 'navigate':
			event.respondWith(navigate(event.request));
			break;
		case 'static':
			event.respondWith(shellFirst(event.request));
			break;
		case 'page':
			event.respondWith(cacheFirst(event, pages));
			break;
		case 'thumbnail':
			event.respondWith(cacheFirst(event, thumbnails));
			break;
		case 'api':
			event.respondWith(networkFirst(event, api));
			break;
		case 'logout':
			event.respondWith(logout(event));
			break;
	}
});

// 退出登录或令牌失效（401）后，这台设备不该再能离线翻看书库，所以清掉缓存的接口结果和图片。
// 网页端自己的文件不含书库数据，留着。请求开始时记下代次，清掉之后才返回的旧请求不再写入。
let generation = 0;
async function forget() {
	generation++;
	await Promise.all([pages, thumbnails, api].map((cache) => cache.clear()));
}

// 写接口和控制台的请求不经过这里，它们收到 401 时由网页端发来消息（`#lib/auth.ts`）。
self.addEventListener('message', (event) => {
	if (event.data === 'forget') event.waitUntil(forget());
});

function forgetOnDenied(event: FetchEvent, response: Response) {
	if (response.status === 401) event.waitUntil(forget());
}

async function logout(event: FetchEvent): Promise<Response> {
	const response = await fetch(event.request);
	if (response.ok) event.waitUntil(forget());
	return response;
}

/** 打开页面总是先问服务端，拿到最新的网页端；连不上时用缓存的 index.html，交给客户端路由。 */
async function navigate(request: Request): Promise<Response> {
	try {
		return await fetch(request);
	} catch (error) {
		const cached = await caches.match('/index.html', { cacheName: SHELL });
		if (cached) return cached;
		throw error;
	}
}

async function shellFirst(request: Request): Promise<Response> {
	return (await caches.match(request, { cacheName: SHELL })) ?? fetch(request);
}

const storable = (response: Response) => response.status === 200 && response.type === 'basic';

function store(event: FetchEvent, cache: BoundedCache, started: number, response: Response) {
	if (!storable(response)) return;
	const url = event.request.url;
	event.waitUntil(
		cache
			.put(url, response.clone(), () => generation === started)
			.catch((error) => console.warn('没能存进离线缓存', url, error))
	);
}

/** 图片地址带内容指纹，有缓存就直接用。 */
async function cacheFirst(event: FetchEvent, cache: BoundedCache): Promise<Response> {
	const cached = await cache.match(event.request.url);
	if (cached) return cached;
	const started = generation;
	// 阅读器翻走时会取消预取，`event.request` 带着它的 signal，取消会一直传到这里的请求。
	const response = await fetch(event.request);
	forgetOnDenied(event, response);
	store(event, cache, started, response);
	return response;
}

/** JSON 先走网络，只在连不上时用上次的结果；服务端返回错误时照常返回错误。 */
async function networkFirst(event: FetchEvent, cache: BoundedCache): Promise<Response> {
	const started = generation;
	try {
		const response = await fetch(event.request);
		forgetOnDenied(event, response);
		store(event, cache, started, response);
		return response;
	} catch (error) {
		const cached = await cache.match(event.request.url);
		if (cached) return cached;
		throw error;
	}
}
