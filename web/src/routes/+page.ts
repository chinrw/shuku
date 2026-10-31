import {
	ApiError,
	categories,
	recent,
	search,
	type ArchiveItem,
	type Category
} from '#lib/api.ts';
import { parseCategory, parseFilter, parseOrder, parseSort } from '#lib/libraryUrl.ts';
import { settings } from '#lib/settings.svelte.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, url }) => {
	const q = url.searchParams.get('q') ?? '';
	const sort = parseSort(url.searchParams.get('sort'));
	const order = parseOrder(url.searchParams.get('o'));
	const category = parseCategory(url.searchParams.get('c'));
	const filter = parseFilter(url.searchParams.get('f'));
	const prefs = settings();
	const group: 'series' | null = prefs.groupSeries && !prefs.ungroupForBatch ? 'series' : null;
	// 分类按钮加载失败不影响列表，只是不显示这一排。
	const shelves: Promise<Category[]> = categories(fetch).catch((error) => {
		console.warn('加载分类失败', error);
		return [];
	});
	// 最近阅读只在书库首页显示。它加载失败不影响列表，只是不显示这一栏。
	const reading: Promise<ArchiveItem[]> = q || category !== null || filter !== null
		? Promise.resolve([])
		: recent(fetch).catch((error) => {
				console.warn('加载最近阅读失败', error);
				return [];
			});
	try {
		// 几个请求互不依赖，并行发出；语句写错或分类不存在时搜索返回 400。
		const [result, categoryList, recentItems] = await Promise.all([
			search(fetch, { q, sort, order, category, filter, group }),
			shelves,
			reading
		]);
		return {
			q,
			sort,
			order,
			category,
			filter,
			group,
			result,
			categories: categoryList,
			recent: recentItems,
			queryError: null
		};
	} catch (error) {
		// 搜索语句写错时留在列表页显示原因，方便直接改；其他错误交给错误页。
		if (error instanceof ApiError && error.status === 400) {
			return {
				q,
				sort,
				order,
				category,
				filter,
				group,
				result: null,
				categories: await shelves,
				recent: [],
				queryError: error.message
			};
		}
		throw error;
	}
};
