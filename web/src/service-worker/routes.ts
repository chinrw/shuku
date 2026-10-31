// Service Worker 按请求的地址决定怎么处理，单独放在这里便于测试。

/**
 * - `navigate`：打开页面，先走网络，离线时用缓存的 index.html。
 * - `page`：原图（含转出的 JPEG），地址带内容指纹，命中缓存就不再请求。
 * - `thumbnail`：封面和页面缩略图，同上，单独一份容量。
 * - `api`：离线时打开书库和阅读器要用的 JSON，先走网络，失败时用上次的结果。
 * - `static`：网页端自己的文件，预缓存过的直接用缓存。
 * - `logout`：退出登录，成功后清掉缓存的书库数据。
 * - `null`：不经过 Service Worker。
 */
export type Route = 'navigate' | 'page' | 'thumbnail' | 'api' | 'static' | 'logout' | null;

const PAGE = /^\/api\/archives\/\d+\/pages\/\d+$/;
const THUMBNAIL = /^\/api\/archives\/\d+(?:\/pages\/\d+)?\/thumbnail$/;

// 只缓存书库页、系列页和阅读器读取的接口。控制台、登录状态、服务器信息、下载和
// 写接口都不缓存，离线时照常报错，免得显示过期的状态或吞掉写入。
const API = [
	/^\/api\/search$/,
	/^\/api\/recent$/,
	/^\/api\/categories$/,
	/^\/api\/saved-tags$/,
	/^\/api\/series\/\d+$/,
	/^\/api\/archives\/\d+$/,
	/^\/api\/archives\/\d+\/pages$/,
	/^\/api\/archives\/\d+\/similar$/
];

export function route(
	request: { method: string; mode: string; url: string },
	origin: string
): Route {
	const url = new URL(request.url);
	if (url.origin !== origin) return null;
	if (request.method === 'DELETE' && url.pathname === '/api/auth/session') return 'logout';
	if (request.method !== 'GET') return null;
	if (request.mode === 'navigate') return 'navigate';
	const path = url.pathname;
	if (path === '/api' || path.startsWith('/api/') || path === '/mcp' || path.startsWith('/mcp/')) {
		// 图片地址的 `v` 是内容指纹，内容变了地址就变，所以可以一直用缓存。
		const versioned = url.searchParams.has('v');
		if (versioned && PAGE.test(path)) return 'page';
		if (versioned && THUMBNAIL.test(path)) return 'thumbnail';
		if (API.some((pattern) => pattern.test(path))) return 'api';
		return null;
	}
	return 'static';
}
