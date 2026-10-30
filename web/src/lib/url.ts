/**
 * 来源 URL 来自 LRR 的标签，内容由抓取插件写入，不可信。
 * 只有 http 和 https 能做成链接，否则 `javascript:` 之类的值会在点击时执行。
 */
export function httpUrl(raw: string): string | null {
	if (!URL.canParse(raw)) return null;
	const url = new URL(raw);
	return url.protocol === 'http:' || url.protocol === 'https:' ? url.href : null;
}
