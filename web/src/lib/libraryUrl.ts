import type { Filter, Order, Sort } from './api';

/** 地址栏 `f=` 里的筛选。“隐藏读完的”是单独的开关，不进地址。 */
export type UrlFilter = Exclude<Filter, 'unfinished'>;

// 地址里认得的筛选值和它们在结果说明里的名字。书库页现在只有“新书”一个按钮，其余几个
// 是早先的按钮，旧链接里的照样生效，结果说明里也照样写出来，免得筛选着却看不出来。
const FILTER_LABELS: Record<UrlFilter, string> = {
	new: '新书',
	unread: '没读过',
	reading: '读了一半',
	finished: '读完',
	untagged: '未打标签'
};

/** 排序下拉框的选项。命名空间排序只列常用的几个，接口接受任意命名空间。 */
export const SORTS: [Sort, string][] = [
	['title', '标题'],
	['date', '加入时间'],
	['pages', '页数'],
	['ns:artist', '作者'],
	['ns:group', '社团'],
	['ns:parody', '原作']
];

/** 不指定方向时的方向：加入时间新的在前，其余从小到大。 */
export function defaultOrder(sort: Sort): Order {
	return sort === 'date' ? 'desc' : 'asc';
}

/** 书库页的地址。默认值（空搜索、按标题、默认方向、不限分类、不筛选）不写进地址栏。 */
export function libraryHref(params: {
	q: string;
	sort: Sort;
	order?: Order | null;
	category: number | null;
	filter?: UrlFilter | null;
}): string {
	const query = new URLSearchParams();
	if (params.q) query.set('q', params.q);
	if (params.sort !== 'title') query.set('sort', params.sort);
	if (params.order && params.order !== defaultOrder(params.sort)) query.set('o', params.order);
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
export function parseFilter(raw: string | null): UrlFilter | null {
	return raw !== null && Object.hasOwn(FILTER_LABELS, raw) ? (raw as UrlFilter) : null;
}

export function filterLabel(filter: UrlFilter): string {
	return FILTER_LABELS[filter];
}

/** 地址里的排序，不认识的值当作按标题。`ns:` 后面是任意命名空间。 */
export function parseSort(raw: string | null): Sort {
	if (raw === 'date' || raw === 'pages') return raw;
	if (raw?.startsWith('ns:') && raw.slice(3).trim()) return raw as Sort;
	return 'title';
}

/** 地址里的方向，不是 asc 或 desc 时当作默认方向。 */
export function parseOrder(raw: string | null): Order | null {
	return raw === 'asc' || raw === 'desc' ? raw : null;
}
