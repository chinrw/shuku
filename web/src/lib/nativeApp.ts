// 桌面和 Android 客户端（仓库的 app/ 目录）直接打开服务器的网页端，并在每个页面里放一个
// `window.shukuApp`，告诉网页端客户端自己的服务器列表页在哪里。浏览器里没有它。

/** 客户端的服务器列表页，带上当前地址，切回这个服务器时从这里继续；不在客户端里时为 `null`。 */
export function serverListHref(current: string, home = globalThis.window?.shukuApp?.home) {
	if (!home) return null;
	const url = new URL(home);
	url.searchParams.set('from', current);
	return url.href;
}

/**
 * 客户端打开服务器时把令牌放在登录页地址的片段里（`/login#token=…`）。片段不会发给服务器，
 * 也不进服务器日志。
 */
export function handoffToken(hash: string): string | null {
	return new URLSearchParams(hash.replace(/^#/, '')).get('token');
}
