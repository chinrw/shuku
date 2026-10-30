import { ApiError, search, type Sort } from '#lib/api.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, url }) => {
	const q = url.searchParams.get('q') ?? '';
	const sort: Sort = url.searchParams.get('sort') === 'date' ? 'date' : 'title';
	try {
		return { q, sort, result: await search(fetch, { q, sort }), queryError: null };
	} catch (error) {
		// 搜索语句写错时留在列表页显示原因，方便直接改；其他错误交给错误页。
		if (error instanceof ApiError && error.status === 400) {
			return { q, sort, result: null, queryError: error.message };
		}
		throw error;
	}
};
