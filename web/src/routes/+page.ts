import {
	ApiError,
	categories,
	recent,
	savedTags,
	search,
	serverInfo,
	type ArchiveItem,
	type Category,
	type Filter
} from '#lib/api.ts';
import {
	parseCategory,
	parseFilter,
	parseOrder,
	parsePage,
	parseSort,
	parseTags
} from '#lib/libraryUrl.ts';
import { savedTagStore } from '#lib/savedTags.svelte.ts';
import { settings } from '#lib/settings.svelte.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, url }) => {
	const q = url.searchParams.get('q') ?? '';
	const prefs = settings();
	// 地址里没写排序时（书库首页、标签链接）用这台设备记住的排序和方向。
	const urlSort = parseSort(url.searchParams.get('sort'));
	const sort = urlSort ?? prefs.sort;
	const order = urlSort ? parseOrder(url.searchParams.get('o')) : prefs.order;
	const page = parsePage(url.searchParams.get('p'));
	const pageSize = prefs.pageSize;
	const category = parseCategory(url.searchParams.get('c'));
	const filter = parseFilter(url.searchParams.get('f'));
	// 筛选行上选中的收藏标签，每个都要完整匹配，和其他条件取交集。
	const tags = parseTags(url.searchParams.getAll('t'));
	// 地址里的筛选加上“隐藏读完的”开关，两者同时生效。
	const filters: Filter[] = [
		...(filter ? [filter] : []),
		...(prefs.hideFinished ? (['unfinished'] as const) : [])
	];
	const group: 'series' | null = prefs.groupSeries && !prefs.ungroupForBatch ? 'series' : null;
	// 分类按钮加载失败不影响列表，只是不显示这一排。
	const shelves: Promise<Category[]> = categories(fetch).catch((error) => {
		console.warn('加载分类失败', error);
		return [];
	});
	// 收藏的标签加载失败不影响列表，只是筛选行上不显示它们。
	const pinnedMark = savedTagStore().mark();
	const pinned: Promise<string[] | null> = savedTags(fetch).catch((error) => {
		console.warn('加载收藏的标签失败', error);
		return null;
	});
	// “新书”按钮上的本数是全库的，和当前搜索无关。取不到时按钮照样能点，只是不显示本数。
	const newBooks: Promise<number | null> = search(fetch, {
		q: '',
		sort: 'title',
		filters: ['new'],
		limit: 1
	}).then(
		(result) => result.total,
		(error) => {
			console.warn('统计新书失败', error);
			return null;
		}
	);
	// 左上角的书库名，取不到时显示默认的“书库”。
	const libraryName: Promise<string> = serverInfo(fetch).then(
		(info) => info.name,
		(error) => {
			console.warn('读取服务器信息失败', error);
			return '书库';
		}
	);
	// 最近阅读只在书库首页显示。它加载失败不影响列表，只是不显示这一栏。
	const reading: Promise<ArchiveItem[]> =
		q || tags.length > 0 || category !== null || filter !== null || page > 1
		? Promise.resolve([])
		: recent(fetch).catch((error) => {
				console.warn('加载最近阅读失败', error);
				return [];
			});
	try {
		// 几个请求互不依赖，并行发出；语句写错或分类不存在时搜索返回 400。
		const [result, categoryList, recentItems, newCount, pinnedTags, name] = await Promise.all([
			search(fetch, {
				q,
				sort,
				order,
				category,
				filters,
				tags,
				group,
				offset: (page - 1) * pageSize,
				limit: pageSize
			}),
			shelves,
			reading,
			newBooks,
			pinned,
			libraryName
		]);
		if (pinnedTags) savedTagStore().set(pinnedTags, pinnedMark);
		return {
			q,
			sort,
			order,
			category,
			filter,
			filters,
			tags,
			page,
			pageSize,
			group,
			result,
			categories: categoryList,
			newCount,
			recent: recentItems,
			libraryName: name,
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
				filters,
				tags,
				page,
				pageSize,
				group,
				result: null,
				categories: await shelves,
				newCount: await newBooks,
				recent: [],
				libraryName: await libraryName,
				queryError: error.message
			};
		}
		throw error;
	}
};
