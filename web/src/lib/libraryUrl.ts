import type { Filter, Sort } from './api';

/** 快捷筛选的按钮文字，也是它们在书库页上的顺序。 */
export const FILTERS: [Filter, string][] = [
	['new', '新书'],
	['unread', '没读过'],
	['reading', '读了一半'],
	['finished', '读完'],
	['untagged', '未打标签']
];

/** 书库页的地址。默认值（空搜索、按标题、不限分类、不筛选）不写进地址栏。 */
export function libraryHref(params: {
	q: string;
	sort: Sort;
	category: number | null;
	filter?: Filter | null;
}): string {
	const query = new URLSearchParams();
	if (params.q) query.set('q', params.q);
	if (params.sort !== 'title') query.set('sort', params.sort);
	if (params.category !== null) query.set('c', String(params.category));
	if (params.filter) query.set('f', params.filter);
	return query.size ? `/?${query}` : '/';
}

/** 地址里的分类 ID，不是正整数时当作没有。 */
export function parseCategory(raw: string | null): number | null {
	const id = Number(raw);
	return raw !== null && Number.isSafeInteger(id) && id > 0 ? id : null;
}

/** 地址里的快捷筛选，不认识的值当作没有。 */
export function parseFilter(raw: string | null): Filter | null {
	return FILTERS.find(([filter]) => filter === raw)?.[0] ?? null;
}
