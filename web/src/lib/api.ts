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
	/** 所属系列，只有一卷的系列不算。 */
	series: SeriesRef | null;
}

export interface SeriesRef {
	id: number;
	name: string;
	volume: number;
	/** 系列一共几卷。 */
	volumes: number;
}

/** 系列页：各卷按卷号排列。 */
export interface Series {
	id: number;
	name: string;
	items: ArchiveItem[];
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

export interface ArchiveDetail extends Omit<ArchiveItem, 'series'> {
	/** 所属系列和前后两卷的 ID，阅读器读完一卷时接着打开下一卷。 */
	series: { id: number; name: string; previous: number | null; next: number | null } | null;
	summary: string;
	file_size: number;
	/** 相对内容目录的路径。 */
	path: string;
	/** 文件已经不在内容目录里，记录可以删除。 */
	missing: boolean;
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

/** `ns:artist` 这类是按命名空间下的标签值排序。 */
export type Sort = 'title' | 'date' | 'pages' | `ns:${string}`;
export type Order = 'asc' | 'desc';

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
		order?: Order | null;
		category?: number | null;
		filter?: Filter | null;
		after?: number;
		limit?: number;
		/** 同一系列只返回一卷，书库页每个系列一张卡。 */
		group?: 'series' | null;
	}
): Promise<SearchResult> {
	const query = new URLSearchParams({ q: params.q, sort: params.sort });
	if (params.order != null) query.set('order', params.order);
	if (params.category != null) query.set('category', String(params.category));
	if (params.filter != null) query.set('filter', params.filter);
	if (params.after !== undefined) query.set('after', String(params.after));
	if (params.limit !== undefined) query.set('limit', String(params.limit));
	if (params.group != null) query.set('group', params.group);
	return request(fetch, `/api/search?${query}`);
}

export function series(fetch: Fetch, id: number): Promise<Series> {
	return request(fetch, `/api/series/${id}`);
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

/** 搜索结果里随机的一本，没有结果时返回 `null`。 */
export async function random(params: {
	q: string;
	category: number | null;
	filter: Filter | null;
}): Promise<number | null> {
	const query = new URLSearchParams({ q: params.q });
	if (params.category !== null) query.set('category', String(params.category));
	if (params.filter !== null) query.set('filter', params.filter);
	try {
		return (await request<{ id: number }>(fetch, `/api/random?${query}`)).id;
	} catch (error) {
		if (error instanceof ApiError && error.status === 404) return null;
		throw error;
	}
}

export type MetadataProvider = 'filename' | 'comicinfo' | 'ehentai';

/** 一次元数据查找写入的内容：替换后的标题（不替换时为空）、加上的标签和新记下的来源。 */
export interface FoundMetadata {
	title: string | null;
	tags: { namespace: string; value: string }[];
	source_url: string | null;
}

/** 对一本书运行一个元数据功能并写入。`url` 只对 E-Hentai 有效，指定画廊不再搜索。 */
export function fetchMetadata(
	archive: number,
	provider: MetadataProvider,
	url?: string
): Promise<FoundMetadata> {
	return request(fetch, `/api/archives/${archive}/metadata`, {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify(url ? { provider, url } : { provider })
	});
}

/** 批量操作要改的书：一串 ID，或者和搜索相同的条件选出的全部结果。 */
export type BatchSelection =
	| { ids: number[] }
	| { search: { q: string; category: number | null; filter: Filter | null } };

export interface BatchChange {
	add_tags?: string[];
	remove_tags?: string[];
	add_to_category?: number;
	remove_from_category?: number;
}

/** 一批在一个事务里改完，返回改了几本。 */
export async function batch(selection: BatchSelection, change: BatchChange): Promise<number> {
	const target =
		'ids' in selection
			? selection
			: {
					search: {
						q: selection.search.q,
						category: selection.search.category ?? undefined,
						filter: selection.search.filter ?? ''
					}
				};
	const result = await request<{ archives: number }>(fetch, '/api/batch', {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify({ ...target, ...change })
	});
	return result.archives;
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

export interface PageList {
	pages: PageInfo[];
	/** 服务端按页面宽高比判断这本是条漫；服务端关掉判断时总是 false。 */
	webtoon: boolean;
}

export function pages(fetch: Fetch, id: number): Promise<PageList> {
	return request(fetch, `/api/archives/${id}/pages`);
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

/** 改标题、简介、来源或标签，省略的字段不变。`source_url` 为空字符串时清除来源。 */
export interface ArchiveEdit {
	title?: string;
	summary?: string;
	source_url?: string;
	/** 整体替换，写法是 `命名空间:值`。 */
	tags?: string[];
}

export function editArchive(id: number, edit: ArchiveEdit): Promise<ArchiveDetail> {
	return request(fetch, `/api/archives/${id}`, {
		method: 'PATCH',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify(edit)
	});
}

/** 删除文件已经不在的记录。文件还在时服务端返回 409。 */
export async function deleteArchive(id: number) {
	await send(fetch, `/api/archives/${id}`, { method: 'DELETE' });
}
