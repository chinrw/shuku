export type ReaderMode = 'paged' | 'scroll';
export type Direction = 'rtl' | 'ltr';

const KEY = 'shuku.reader';

class ReaderSettings {
	mode = $state<ReaderMode>('paged');
	direction = $state<Direction>('rtl');

	constructor() {
		const saved = JSON.parse(localStorage.getItem(KEY) ?? '{}');
		if (saved.mode === 'paged' || saved.mode === 'scroll') this.mode = saved.mode;
		if (saved.direction === 'rtl' || saved.direction === 'ltr') this.direction = saved.direction;
	}

	save() {
		localStorage.setItem(KEY, JSON.stringify({ mode: this.mode, direction: this.direction }));
	}
}

let settings: ReaderSettings | undefined;

/** 第一次调用时才读 localStorage，模块本身可以在构建分析时被安全导入。 */
export function readerSettings(): ReaderSettings {
	settings ??= new ReaderSettings();
	return settings;
}
