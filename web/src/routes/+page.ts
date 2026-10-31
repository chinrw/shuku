import { ApiError, facets, recent, search, type ArchiveItem, type Sort } from '#lib/api.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, url }) => {
	const q = url.searchParams.get('q') ?? '';
	const sort: Sort = url.searchParams.get('sort') === 'date' ? 'date' : 'title';
	// 最近阅读只在书库首页显示。它加载失败不影响列表，只是不显示这一栏。
	const reading: Promise<ArchiveItem[]> = q
		? Promise.resolve([])
		: recent(fetch).catch((error) => {
				console.warn('加载最近阅读失败', error);
				return [];
			});
	try {
		// 几个请求互不依赖，并行发出；语句写错时搜索和标签统计都返回 400。
		const [result, tags, recentItems] = await Promise.all([
			search(fetch, { q, sort }),
			facets(fetch, q),
			reading
		]);
		return { q, sort, result, facets: tags, recent: recentItems, queryError: null };
	} catch (error) {
		// 搜索语句写错时留在列表页显示原因，方便直接改；其他错误交给错误页。
		if (error instanceof ApiError && error.status === 400) {
			return { q, sort, result: null, facets: [], recent: [], queryError: error.message };
		}
		throw error;
	}
};
