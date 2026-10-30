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
	date_added: number;
	source_url: string | null;
	file_size: number;
	tags: TagGroup[];
}

export interface PageInfo {
	width: number;
	height: number;
	url: string;
}

export type Sort = 'title' | 'date';

export class ApiError extends Error {
	constructor(
		readonly status: number,
		message: string
	) {
		super(message);
	}
}

type Fetch = typeof fetch;

async function request<T>(fetch: Fetch, path: string, init?: RequestInit): Promise<T> {
	const response = await fetch(path, init);
	if (!response.ok) {
		// 服务端的错误统一是 {"error": "..."}；代理或网络层的错误可能不是 JSON。
		const body = await response.json().catch(() => null);
		throw new ApiError(response.status, body?.error ?? response.statusText);
	}
	return response.json();
}

export function search(
	fetch: Fetch,
	params: { q: string; sort: Sort; after?: number; limit?: number }
): Promise<SearchResult> {
	const query = new URLSearchParams({ q: params.q, sort: params.sort });
	if (params.after !== undefined) query.set('after', String(params.after));
	if (params.limit !== undefined) query.set('limit', String(params.limit));
	return request(fetch, `/api/search?${query}`);
}

export function archive(fetch: Fetch, id: number): Promise<ArchiveDetail> {
	return request(fetch, `/api/archives/${id}`);
}

export async function pages(fetch: Fetch, id: number): Promise<PageInfo[]> {
	return (await request<{ pages: PageInfo[] }>(fetch, `/api/archives/${id}/pages`)).pages;
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
