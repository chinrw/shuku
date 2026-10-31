// 服务器列表的纯逻辑，不碰 DOM，`servers.test.js` 用 node --test 测。

/** @typedef {{ id: string, name: string, url: string, token: string | null, lastUrl: string | null }} Server */
/** @typedef {{ name: string, version: string, archives: number, require_token: boolean }} ServerInfo */

/**
 * 把用户填的地址整理成源（协议、主机和端口）。没写协议时按 http，局域网里直连的服务器都是 http。
 * @param {string} input
 * @returns {string}
 */
export function normalizeUrl(input) {
	const text = input.trim();
	if (!text) throw new Error('请填服务器地址');
	let url;
	try {
		url = new URL(/^[a-z][a-z0-9+.-]*:\/\//i.test(text) ? text : `http://${text}`);
	} catch {
		throw new Error('地址格式不对');
	}
	if (url.protocol !== 'http:' && url.protocol !== 'https:') {
		throw new Error('地址要以 http:// 或 https:// 开头');
	}
	return url.origin;
}

/**
 * 从服务器的网页端回到列表时，记下当时的页面，切回这个服务器时从这里继续。
 * @param {Server[]} servers
 * @param {string} from
 * @returns {{ servers: Server[], current: string | null }}
 */
export function rememberPage(servers, from) {
	let origin;
	try {
		origin = new URL(from).origin;
	} catch {
		return { servers, current: null };
	}
	const match = servers.find((server) => server.url === origin);
	if (!match) return { servers, current: null };
	return {
		servers: servers.map((server) => (server === match ? { ...server, lastUrl: from } : server)),
		current: match.id
	};
}

/**
 * 打开服务器时要跳去的地址。有令牌时先经过登录页，令牌放在片段里，不发给服务器的日志；
 * 登录后回到上次的页面。
 * @param {Server} server
 * @returns {string}
 */
export function openTarget(server) {
	const last = server.lastUrl ? new URL(server.lastUrl) : null;
	const path = last && last.origin === server.url ? last.pathname + last.search + last.hash : '/';
	if (!server.token) return server.url + path;
	const query = new URLSearchParams({ next: path });
	const fragment = new URLSearchParams({ token: server.token });
	return `${server.url}/login?${query}#${fragment}`;
}

/**
 * @param {Server[]} servers
 * @param {Server} server
 * @returns {Server[]}
 */
export function upsert(servers, server) {
	return servers.some((item) => item.id === server.id)
		? servers.map((item) => (item.id === server.id ? server : item))
		: [...servers, server];
}

/**
 * 读 `/api/info`。服务器不在线或者不是 Shuku 时抛出错误，消息给人看。
 * @param {string} origin
 * @param {typeof fetch} [fetcher]
 * @returns {Promise<ServerInfo>}
 */
export async function fetchInfo(origin, fetcher = fetch) {
	let response;
	try {
		response = await fetcher(`${origin}/api/info`, { signal: AbortSignal.timeout(5000) });
	} catch {
		throw new Error('连不上这个地址');
	}
	if (!response.ok) throw new Error(`服务器返回 ${response.status}，可能不是 Shuku 或者版本太旧`);
	const info = await response.json().catch(() => null);
	if (!info || typeof info.name !== 'string' || typeof info.require_token !== 'boolean') {
		throw new Error('这个地址不是 Shuku 服务器');
	}
	return info;
}
