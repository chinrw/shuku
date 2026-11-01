/** 书库每页可选的本数，搜索接口一次最多返回 200 本。 */
export const PAGE_SIZES = [50, 100, 200] as const;
export type PageSize = (typeof PAGE_SIZES)[number];
export const DEFAULT_PAGE_SIZE: PageSize = 100;

export function isPageSize(value: unknown): value is PageSize {
	return PAGE_SIZES.includes(value as PageSize);
}

/**
 * 页码条上显示的页：第一页、最后一页和当前页前后两页，中间断开的地方用 `null` 表示省略号。
 * `total` 是总页数，至少 1。
 */
export function pageNumbers(current: number, total: number): (number | null)[] {
	// 地址里的页码可以超过总页数，按最后一页算附近的页。
	const middle = Math.min(Math.max(current, 1), total);
	const shown = new Set([1, total]);
	for (const delta of [-2, -1, 0, 1, 2]) {
		const page = middle + delta;
		if (page >= 1 && page <= total) shown.add(page);
	}
	const sorted = [...shown].sort((a, b) => a - b);
	const result: (number | null)[] = [];
	for (const page of sorted) {
		const last = result.at(-1);
		if (typeof last === 'number' && page - last > 1) result.push(page - last === 2 ? last + 1 : null);
		result.push(page);
	}
	return result;
}

/** 总页数，没有结果时也算一页。 */
export function pageCount(total: number, size: PageSize): number {
	return Math.max(1, Math.ceil(total / size));
}
