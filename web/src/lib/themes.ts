/*
 * 网页端的全部配色。每套配色给出 app.css 里用到的全部颜色变量，由根布局写到 <html> 上，
 * 加一套配色只要在 THEMES 里加一项。
 *
 * ExHentai 深色和 E-Hentai 浅色取自 LRR 自带的两套主题；Tokyo Night 和 Catppuccin 用官方调色板，
 * 只有 Latte 的“新”和作者、社团、原作几种颜色在官方色的基础上调深，否则在浅底上看不清。
 * themes.test.ts 检查每套配色的文字对比度。
 */

export type Scheme = 'dark' | 'light';

export const TOKENS = [
	'bg',
	'surface',
	// 比底色更深一层，控制台的侧栏、日志、进度条底槽和阅读器工具栏用。
	'sunken',
	'raised',
	'chip',
	'chip-border',
	'fg',
	'muted',
	'faint',
	'border',
	'link',
	'accent',
	// 主按钮、“新”角标这些用 accent、new、danger 填色的地方上面的字。浅色强调色上用深色字。
	'on-accent',
	'new',
	'danger',
	'artist',
	'group',
	'parody'
] as const;

export type Token = (typeof TOKENS)[number];

export interface Theme {
	id: string;
	label: string;
	/** 设置里按系列分组，空字符串不分组。 */
	family: string;
	scheme: Scheme;
	colors: Record<Token, string>;
}

/** 跟随系统时默认的深色和浅色配色。 */
export const DEFAULT_DARK = 'exhentai';
export const DEFAULT_LIGHT = 'ehentai';

const tokyoNight = (base: { bg: string; sunken: string; surface: string }) => ({
	...base,
	raised: '#3b4261',
	chip: '#3b4261',
	'chip-border': '#545c7e',
	fg: '#c0caf5',
	muted: '#a9b1d6',
	faint: '#737aa2',
	border: '#3b4261',
	link: '#7aa2f7',
	accent: '#7aa2f7',
	'on-accent': base.sunken,
	new: '#ff9e64',
	danger: '#f7768e',
	artist: '#7dcfff',
	group: '#73daca',
	parody: '#bb9af7'
});

/** Catppuccin 深色三版的调色板结构相同，按官方的名字取色。 */
const catppuccinDark = (p: {
	base: string;
	mantle: string;
	crust: string;
	surface0: string;
	surface1: string;
	surface2: string;
	overlay1: string;
	subtext0: string;
	text: string;
	blue: string;
	peach: string;
	red: string;
	sapphire: string;
	teal: string;
	pink: string;
}) => ({
	bg: p.base,
	surface: p.surface0,
	sunken: p.mantle,
	raised: p.surface0,
	chip: p.surface1,
	'chip-border': p.surface2,
	fg: p.text,
	muted: p.subtext0,
	faint: p.overlay1,
	border: p.surface1,
	link: p.blue,
	accent: p.blue,
	'on-accent': p.crust,
	new: p.peach,
	danger: p.red,
	artist: p.sapphire,
	group: p.teal,
	parody: p.pink
});

export const THEMES: readonly Theme[] = [
	{
		id: 'exhentai',
		label: 'ExHentai 深色',
		family: '',
		scheme: 'dark',
		colors: {
			bg: '#34353b',
			surface: '#43464e',
			sunken: '#2b2c31',
			raised: '#4b4f57',
			chip: '#4f535b',
			'chip-border': '#6d7179',
			fg: '#dddddd',
			muted: '#a6a9ae',
			faint: '#7c8087',
			border: '#50545c',
			link: '#3b97ea',
			accent: '#3b97ea',
			'on-accent': '#ffffff',
			new: '#e8833a',
			danger: '#f87171',
			artist: '#22a7f0',
			group: '#36d7b7',
			parody: '#d2527f'
		}
	},
	{
		id: 'ehentai',
		label: 'E-Hentai 浅色',
		family: '',
		scheme: 'light',
		colors: {
			bg: '#e3e0d1',
			surface: '#edebdf',
			sunken: '#d6d1bd',
			raised: '#f2efdf',
			chip: '#f2efdf',
			'chip-border': '#806769',
			fg: '#5c0d11',
			muted: '#806769',
			faint: '#a8999a',
			border: '#cfcab6',
			link: '#9b4e03',
			accent: '#9b4e03',
			'on-accent': '#ffffff',
			new: '#c4561b',
			danger: '#b91c1c',
			artist: '#0f6ba8',
			group: '#0e7f69',
			parody: '#a8325e'
		}
	},
	{
		id: 'graphite',
		label: '石墨',
		family: '',
		scheme: 'dark',
		colors: {
			bg: '#1c1d20',
			surface: '#26272b',
			sunken: '#161719',
			raised: '#2f3035',
			chip: '#34353a',
			'chip-border': '#55565c',
			fg: '#e6e6e6',
			muted: '#a3a3a8',
			faint: '#75767c',
			border: '#3a3b40',
			link: '#f0a13a',
			accent: '#f0a13a',
			'on-accent': '#1c1d20',
			new: '#f97316',
			danger: '#f87171',
			artist: '#22a7f0',
			group: '#36d7b7',
			parody: '#e0608e'
		}
	},
	{
		id: 'tokyonight-night',
		label: 'Tokyo Night',
		family: 'Tokyo Night',
		scheme: 'dark',
		colors: tokyoNight({ bg: '#1a1b26', sunken: '#16161e', surface: '#292e42' })
	},
	{
		id: 'tokyonight-storm',
		label: 'Tokyo Night Storm',
		family: 'Tokyo Night',
		scheme: 'dark',
		colors: tokyoNight({ bg: '#24283b', sunken: '#1f2335', surface: '#292e42' })
	},
	{
		id: 'tokyonight-moon',
		label: 'Tokyo Night Moon',
		family: 'Tokyo Night',
		scheme: 'dark',
		colors: {
			bg: '#222436',
			surface: '#2f334d',
			sunken: '#1e2030',
			raised: '#3b4261',
			chip: '#3b4261',
			'chip-border': '#545c7e',
			fg: '#c8d3f5',
			muted: '#828bb8',
			faint: '#737aa2',
			border: '#3b4261',
			link: '#82aaff',
			accent: '#82aaff',
			'on-accent': '#1e2030',
			new: '#ff966c',
			danger: '#ff757f',
			artist: '#86e1fc',
			group: '#4fd6be',
			parody: '#c099ff'
		}
	},
	{
		id: 'tokyonight-day',
		label: 'Tokyo Night Day',
		family: 'Tokyo Night',
		scheme: 'light',
		colors: {
			bg: '#e1e2e7',
			surface: '#e9e9ed',
			sunken: '#d0d5e3',
			raised: '#ececf0',
			chip: '#ececf0',
			'chip-border': '#8990b3',
			fg: '#3760bf',
			muted: '#6172b0',
			faint: '#848cb5',
			border: '#c4c8da',
			link: '#2e7de9',
			accent: '#2e7de9',
			'on-accent': '#ffffff',
			new: '#b15c00',
			danger: '#c64343',
			artist: '#007197',
			group: '#387068',
			parody: '#7847bd'
		}
	},
	{
		id: 'catppuccin-latte',
		label: 'Catppuccin Latte',
		family: 'Catppuccin',
		scheme: 'light',
		colors: {
			bg: '#eff1f5',
			surface: '#e6e9ef',
			sunken: '#dce0e8',
			raised: '#e6e9ef',
			chip: '#e6e9ef',
			'chip-border': '#9ca0b0',
			fg: '#4c4f69',
			muted: '#6c6f85',
			faint: '#8c8fa1',
			border: '#ccd0da',
			link: '#1e66f5',
			accent: '#1e66f5',
			'on-accent': '#eff1f5',
			// 官方的 peach、sapphire、teal、pink 在 base 上对比度不到 3，按原色相调深。
			new: '#c94f06',
			danger: '#d20f39',
			artist: '#13707f',
			group: '#0f6e73',
			parody: '#b3348f'
		}
	},
	{
		id: 'catppuccin-frappe',
		label: 'Catppuccin Frappé',
		family: 'Catppuccin',
		scheme: 'dark',
		colors: catppuccinDark({
			base: '#303446',
			mantle: '#292c3c',
			crust: '#232634',
			surface0: '#414559',
			surface1: '#51576d',
			surface2: '#626880',
			overlay1: '#838ba7',
			subtext0: '#a5adce',
			text: '#c6d0f5',
			blue: '#8caaee',
			peach: '#ef9f76',
			red: '#e78284',
			sapphire: '#85c1dc',
			teal: '#81c8be',
			pink: '#f4b8e4'
		})
	},
	{
		id: 'catppuccin-macchiato',
		label: 'Catppuccin Macchiato',
		family: 'Catppuccin',
		scheme: 'dark',
		colors: catppuccinDark({
			base: '#24273a',
			mantle: '#1e2030',
			crust: '#181926',
			surface0: '#363a4f',
			surface1: '#494d64',
			surface2: '#5b6078',
			overlay1: '#8087a2',
			subtext0: '#a5adcb',
			text: '#cad3f5',
			blue: '#8aadf4',
			peach: '#f5a97f',
			red: '#ed8796',
			sapphire: '#7dc4e4',
			teal: '#8bd5ca',
			pink: '#f5bde6'
		})
	},
	{
		id: 'catppuccin-mocha',
		label: 'Catppuccin Mocha',
		family: 'Catppuccin',
		scheme: 'dark',
		colors: catppuccinDark({
			base: '#1e1e2e',
			mantle: '#181825',
			crust: '#11111b',
			surface0: '#313244',
			surface1: '#45475a',
			surface2: '#585b70',
			overlay1: '#7f849c',
			subtext0: '#a6adc8',
			text: '#cdd6f4',
			blue: '#89b4fa',
			peach: '#fab387',
			red: '#f38ba8',
			sapphire: '#74c7ec',
			teal: '#94e2d5',
			pink: '#f5c2e7'
		})
	}
];

/** 设置里的“配色”：跟随系统，或者固定用某一套。 */
export type ThemeChoice = 'system' | string;

export function findTheme(id: string, scheme?: Scheme): Theme | undefined {
	return THEMES.find((theme) => theme.id === id && (!scheme || theme.scheme === scheme));
}

/** 按设置和系统的深浅色决定当前用哪一套。找不到的编号（例如删掉了的配色）退回默认。 */
export function activeThemes(
	choice: ThemeChoice,
	dark: string,
	light: string
): { dark: Theme; light: Theme } {
	const fixed = choice === 'system' ? undefined : findTheme(choice);
	return {
		dark: fixed ?? findTheme(dark, 'dark') ?? findTheme(DEFAULT_DARK)!,
		light: fixed ?? findTheme(light, 'light') ?? findTheme(DEFAULT_LIGHT)!
	};
}

/** 设置页的选项，按系列分组，保持 THEMES 里的顺序。 */
export function themeGroups(scheme?: Scheme): { family: string; themes: Theme[] }[] {
	const groups: { family: string; themes: Theme[] }[] = [];
	for (const theme of THEMES) {
		if (scheme && theme.scheme !== scheme) continue;
		const last = groups.at(-1);
		if (last?.family === theme.family) last.themes.push(theme);
		else groups.push({ family: theme.family, themes: [theme] });
	}
	return groups;
}

/**
 * app.html 里的内联脚本在网页端的代码加载前读这一项，把上次的配色先套上，免得打开时闪一下默认色。
 * 内容是跟随系统时深色和浅色各用的那一套；固定配色时两项相同。
 */
export const BOOT_KEY = 'shuku.theme';

export function applyTheme(theme: Theme, pair: { dark: Theme; light: Theme }) {
	const root = document.documentElement;
	root.dataset.theme = theme.id;
	root.style.colorScheme = theme.scheme;
	for (const token of TOKENS) root.style.setProperty(`--${token}`, theme.colors[token]);
	// app.html 里按系统深浅色各有一个 theme-color，选了固定配色时两个都换成它的底色。
	for (const meta of document.querySelectorAll('meta[name="theme-color"]')) {
		const scheme = meta.getAttribute('media')?.includes('light') ? 'light' : 'dark';
		meta.setAttribute('content', pair[scheme].colors.bg);
	}
	try {
		const boot = (entry: Theme) => ({ id: entry.id, scheme: entry.scheme, colors: entry.colors });
		localStorage.setItem(BOOT_KEY, JSON.stringify({ dark: boot(pair.dark), light: boot(pair.light) }));
	} catch {
		// 存不下（隐私模式）时只是下次打开会闪一下。
	}
}
