type Size = { width: number; height: number };

/**
 * 双页模式下每一屏显示的页码。封面单独一屏；宽大于高的页面本身就是跨页图，
 * 也单独一屏；其余相邻两页成对。
 */
export function buildSpreads(pages: Size[]): number[][] {
	const wide = (page: Size) => page.width > page.height;
	const spreads: number[][] = [];
	let index = 0;
	while (index < pages.length) {
		const pairable =
			index > 0 && index + 1 < pages.length && !wide(pages[index]) && !wide(pages[index + 1]);
		spreads.push(pairable ? [index, index + 1] : [index]);
		index += pairable ? 2 : 1;
	}
	return spreads;
}
