import type { Sort } from './api';

/** 书库页的地址。默认值（空搜索、按标题、不限分类）不写进地址栏。 */
export function libraryHref(params: { q: string; sort: Sort; category: number | null }): string {
	const query = new URLSearchParams();
	if (params.q) query.set('q', params.q);
	if (params.sort !== 'title') query.set('sort', params.sort);
	if (params.category !== null) query.set('c', String(params.category));
	return query.size ? `/?${query}` : '/';
}

/** 地址里的分类 ID，不是正整数时当作没有。 */
export function parseCategory(raw: string | null): number | null {
	const id = Number(raw);
	return raw !== null && Number.isSafeInteger(id) && id > 0 ? id : null;
}
