// 接口格式见仓库的 docs/api.md。

export interface Progress {
	page: number;
	client_time: number;
}

export interface ArchiveItem {
	id: number;
	title: string;
	page_count: number;
	is_new: boolean;
	progress: Progress | null;
	cover_url: string;
	/** Unix 秒。 */
	date_added: number;
	source_url: string | null;
	tags: TagGroup[];
}

export interface SearchResult {
	total: number;
	next_after: number | null;
	degraded: boolean;
	items: ArchiveItem[];
}

export interface TagGroup {
	namespace: string;
	values: string[];
}

export interface ArchiveDetail extends ArchiveItem {
	summary: string;
	file_size: number;
	/** 这本书所在的静态分类的 ID。 */
	categories: number[];
}

export interface PageInfo {
	width: number;
	height: number;
	url: string;
	/** 缩到 500 宽的 JPEG，详情页的页面总览用。 */
	thumbnail_url: string;
}

export type Sort = 'title' | 'date';

/** 书库页的快捷筛选，见 docs/api.md 的搜索接口。 */
export type Filter = 'new' | 'unread' | 'reading' | 'finished' | 'untagged';

export interface Category {
	id: number;
	name: string;
	pinned: boolean;
	/** 动态分类的成员是一条搜索语句的结果。 */
	dynamic: boolean;
	count: number;
}

/** `tag` 是小写的完整标签名，例如 `artist:某某`。 */
export interface TagCount {
	tag: string;
	count: number;
}

export class ApiError extends Error {
	constructor(
		readonly status: number,
		message: string
	) {
		super(message);
	}
}

type Fetch = typeof fetch;

async function send(fetch: Fetch, path: string, init?: RequestInit): Promise<Response> {
	const response = await fetch(path, init);
	if (!response.ok) {
		// 服务端的错误统一是 {"error": "..."}；代理或网络层的错误可能不是 JSON。
		const body = await response.json().catch(() => null);
		throw new ApiError(response.status, body?.error ?? response.statusText);
	}
	return response;
}

async function request<T>(fetch: Fetch, path: string, init?: RequestInit): Promise<T> {
	return (await send(fetch, path, init)).json();
}

export function search(
	fetch: Fetch,
	params: {
		q: string;
		sort: Sort;
		category?: number | null;
		filter?: Filter | null;
		after?: number;
		limit?: number;
	}
): Promise<SearchResult> {
	const query = new URLSearchParams({ q: params.q, sort: params.sort });
	if (params.category != null) query.set('category', String(params.category));
	if (params.filter != null) query.set('filter', params.filter);
	if (params.after !== undefined) query.set('after', String(params.after));
	if (params.limit !== undefined) query.set('limit', String(params.limit));
	return request(fetch, `/api/search?${query}`);
}

/** 书库页的分类按钮，置顶的在前。 */
export async function categories(fetch: Fetch): Promise<Category[]> {
	return (await request<{ categories: Category[] }>(fetch, '/api/categories')).categories;
}

/** 新建分类，`search` 为空是静态分类。 */
export function createCategory(name: string, search = ''): Promise<{ id: number }> {
	return request(fetch, '/api/categories', {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify({ name, search })
	});
}

/** 把归档加进静态分类或移出。 */
export async function setCategoryMember(category: number, archive: number, member: boolean) {
	await send(fetch, `/api/categories/${category}/archives/${archive}`, {
		method: member ? 'PUT' : 'DELETE'
	});
}

/** 最近读过的归档，按翻页时间从新到旧。 */
export async function recent(fetch: Fetch, limit?: number): Promise<ArchiveItem[]> {
	const query = limit === undefined ? '' : `?${new URLSearchParams({ limit: String(limit) })}`;
	return (await request<{ items: ArchiveItem[] }>(fetch, `/api/recent${query}`)).items;
}

export async function facets(fetch: Fetch, q: string): Promise<TagCount[]> {
	const query = new URLSearchParams({ q });
	return (await request<{ tags: TagCount[] }>(fetch, `/api/facets?${query}`)).tags;
}

export async function suggestTags(
	prefix: string,
	signal: AbortSignal,
	limit?: number
): Promise<TagCount[]> {
	const query = new URLSearchParams({ prefix });
	if (limit !== undefined) query.set('limit', String(limit));
	return (await request<{ tags: TagCount[] }>(fetch, `/api/tags?${query}`, { signal })).tags;
}

export function archive(fetch: Fetch, id: number): Promise<ArchiveDetail> {
	return request(fetch, `/api/archives/${id}`);
}

export async function pages(fetch: Fetch, id: number): Promise<PageInfo[]> {
	return (await request<{ pages: PageInfo[] }>(fetch, `/api/archives/${id}/pages`)).pages;
}

/** 原始归档文件，响应带 `Content-Disposition: attachment`。 */
export function downloadUrl(id: number): string {
	return `/api/archives/${id}/file`;
}

export function putProgress(
	id: number,
	progress: Progress,
	options: { keepalive?: boolean } = {}
): Promise<{ accepted: boolean; progress: Progress }> {
	return request(fetch, `/api/archives/${id}/progress`, {
		method: 'PUT',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify(progress),
		keepalive: options.keepalive
	});
}
