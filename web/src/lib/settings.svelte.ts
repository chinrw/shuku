export type ReaderMode = 'paged' | 'scroll';
export type Direction = 'rtl' | 'ltr';
export type LibraryView = 'grid' | 'list';

const KEY = 'shuku.settings';

class Settings {
	mode = $state<ReaderMode>('paged');
	direction = $state<Direction>('rtl');
	/** 横屏时把相邻两页并排显示。 */
	spread = $state(true);
	view = $state<LibraryView>('grid');

	constructor() {
		const saved = JSON.parse(localStorage.getItem(KEY) ?? '{}');
		if (saved.mode === 'paged' || saved.mode === 'scroll') this.mode = saved.mode;
		if (saved.direction === 'rtl' || saved.direction === 'ltr') this.direction = saved.direction;
		if (typeof saved.spread === 'boolean') this.spread = saved.spread;
		if (saved.view === 'grid' || saved.view === 'list') this.view = saved.view;
	}

	save() {
		const { mode, direction, spread, view } = this;
		localStorage.setItem(KEY, JSON.stringify({ mode, direction, spread, view }));
	}
}

let instance: Settings | undefined;

/** 第一次调用时才读 localStorage，模块本身可以在构建分析时被安全导入。 */
export function settings(): Settings {
	instance ??= new Settings();
	return instance;
}
