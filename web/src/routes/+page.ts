import { ApiError, facets, search, type Sort } from '#lib/api.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, url }) => {
	const q = url.searchParams.get('q') ?? '';
	const sort: Sort = url.searchParams.get('sort') === 'date' ? 'date' : 'title';
	try {
		// 两个请求互不依赖，并行发出；语句写错时两者都返回 400。
		const [result, tags] = await Promise.all([search(fetch, { q, sort }), facets(fetch, q)]);
		return { q, sort, result, facets: tags, queryError: null };
	} catch (error) {
		// 搜索语句写错时留在列表页显示原因，方便直接改；其他错误交给错误页。
		if (error instanceof ApiError && error.status === 400) {
			return { q, sort, result: null, facets: [], queryError: error.message };
		}
		throw error;
	}
};
