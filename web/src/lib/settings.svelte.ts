import type { Order, Sort } from './api';
import { parseOrder, parseSort } from './libraryUrl';
import { DEFAULT_PAGE_SIZE, isPageSize, type PageSize } from './pages';
import { DEFAULT_DARK, DEFAULT_LIGHT, findTheme, type ThemeChoice } from './themes';

export type ReaderMode = 'paged' | 'scroll';
export type Direction = 'rtl' | 'ltr';
/** 翻页模式下整页放进屏幕，或者页宽占满屏幕、上下滚动。 */
export type Fit = 'screen' | 'width';
export type LibraryView = 'grid' | 'list';
/** 手机和平板上阅读器按屏幕取缩小的图（放大时换原图），或者总用原图。 */
export type Quality = 'auto' | 'original';

const KEY = 'shuku.settings';

/** 条漫宽度滑块的范围和默认值。 */
export const WEBTOON_WIDTH = { min: 480, max: 1200, step: 40, default: 800 } as const;

class Settings {
	mode = $state<ReaderMode>('paged');
	direction = $state<Direction>('rtl');
	fit = $state<Fit>('screen');
	/** 横屏时把相邻两页并排显示。 */
	spread = $state(true);
	quality = $state<Quality>('auto');
	/** 翻页模式里用滚轮翻页。 */
	wheelTurn = $state(true);
	view = $state<LibraryView>('grid');
	/** 书库页同一系列的各卷合成一张卡。 */
	groupSeries = $state(true);
	/** 书库的排序，地址里没写排序时用它。默认按加入时间、最新的在前。 */
	sort = $state<Sort>('date');
	/** `null` 是这种排序的默认方向。 */
	order = $state<Order | null>(null);
	/** 书库页不显示读到最后一页的书，和别的筛选同时生效。 */
	hideFinished = $state(false);
	/** 书库每页的本数。 */
	pageSize = $state<PageSize>(DEFAULT_PAGE_SIZE);
	/**
	 * 批量操作时临时展开各卷，不保存：系列卡只代表一卷，按它选择或按合并后的总数全选，
	 * 都和服务端实际修改的书对不上。
	 */
	ungroupForBatch = false;
	/** 电脑和 iPad 横屏上条漫那一栏的最大宽度（CSS 像素）。 */
	webtoonWidth = $state<number>(WEBTOON_WIDTH.default);
	/** 手动指定的条漫开关，按归档 ID 记。没有记录的书按页面宽高比自动判断。 */
	webtoon = $state<Record<number, boolean>>({});
	/** 配色，见 themes.ts。跟随系统时深色和浅色各用 darkTheme、lightTheme。 */
	theme = $state<ThemeChoice>('system');
	darkTheme = $state<string>(DEFAULT_DARK);
	lightTheme = $state<string>(DEFAULT_LIGHT);

	constructor() {
		const saved = load();
		if (saved.mode === 'paged' || saved.mode === 'scroll') this.mode = saved.mode;
		if (saved.direction === 'rtl' || saved.direction === 'ltr') this.direction = saved.direction;
		if (saved.fit === 'screen' || saved.fit === 'width') this.fit = saved.fit;
		if (typeof saved.spread === 'boolean') this.spread = saved.spread;
		if (saved.quality === 'auto' || saved.quality === 'original') this.quality = saved.quality;
		if (typeof saved.wheelTurn === 'boolean') this.wheelTurn = saved.wheelTurn;
		if (saved.view === 'grid' || saved.view === 'list') this.view = saved.view;
		if (typeof saved.groupSeries === 'boolean') this.groupSeries = saved.groupSeries;
		if (typeof saved.hideFinished === 'boolean') this.hideFinished = saved.hideFinished;
		if (isPageSize(saved.pageSize)) this.pageSize = saved.pageSize;
		const sort = typeof saved.sort === 'string' ? parseSort(saved.sort) : null;
		if (sort) {
			this.sort = sort;
			this.order = parseOrder(saved.order ?? null);
		}
		if (
			typeof saved.webtoonWidth === 'number' &&
			saved.webtoonWidth >= WEBTOON_WIDTH.min &&
			saved.webtoonWidth <= WEBTOON_WIDTH.max
		) {
			this.webtoonWidth = saved.webtoonWidth;
		}
		if (saved.theme === 'system' || findTheme(saved.theme)) this.theme = saved.theme;
		if (findTheme(saved.darkTheme, 'dark')) this.darkTheme = saved.darkTheme;
		if (findTheme(saved.lightTheme, 'light')) this.lightTheme = saved.lightTheme;
		if (saved.webtoon && typeof saved.webtoon === 'object') {
			for (const [id, value] of Object.entries(saved.webtoon)) {
				if (typeof value === 'boolean') this.webtoon[Number(id)] = value;
			}
		}
	}

	save() {
		const { mode, direction, fit, spread, view, groupSeries, hideFinished, webtoonWidth, webtoon } =
			this;
		const { sort, order, quality, wheelTurn, pageSize, theme, darkTheme, lightTheme } = this;
		localStorage.setItem(
			KEY,
			JSON.stringify({
				mode,
				direction,
				fit,
				spread,
				quality,
				wheelTurn,
				view,
				groupSeries,
				hideFinished,
				pageSize,
				sort,
				order,
				webtoonWidth,
				webtoon,
				theme,
				darkTheme,
				lightTheme
			})
		);
	}
}

// 根布局一打开就读设置，存储被禁用或内容损坏时用默认值，不能让整个网页端打不开。
function load(): Record<string, any> {
	try {
		const saved = JSON.parse(localStorage.getItem(KEY) ?? '{}');
		return saved && typeof saved === 'object' ? saved : {};
	} catch {
		return {};
	}
}

let instance: Settings | undefined;

/** 第一次调用时才读 localStorage，模块本身可以在构建分析时被安全导入。 */
export function settings(): Settings {
	instance ??= new Settings();
	return instance;
}
