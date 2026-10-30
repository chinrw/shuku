// 服务端按请求的宽度缓存缩放结果，只用固定几档宽度可以让缓存变体保持在少数几个。
export const WIDTHS = [720, 1080, 1440, 2160] as const;

/** 选出能覆盖设备像素宽度的最小一档；超过最大档时用最大档。 */
export function widthTier(cssWidth: number, devicePixelRatio: number): number {
	const needed = cssWidth * devicePixelRatio;
	return WIDTHS.find((width) => width >= needed) ?? WIDTHS[WIDTHS.length - 1];
}

/** 原图不比这一档宽时直接用原图，不让服务端生成一份同尺寸的 JPEG。 */
export function pageUrl(page: { width: number; url: string }, tier: number): string {
	return tier >= page.width ? page.url : `${page.url}&w=${tier}`;
}

/** 整页放进视口时的显示宽度，同时受视口宽和高限制。 */
export function fittedWidth(
	page: { width: number; height: number },
	viewport: { width: number; height: number }
): number {
	return Math.min(viewport.width, (viewport.height * page.width) / page.height);
}
