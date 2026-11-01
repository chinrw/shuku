import { describe, expect, test } from 'vitest';
import { activeThemes, DEFAULT_DARK, DEFAULT_LIGHT, findTheme, themeGroups, THEMES, TOKENS } from './themes';

describe('THEMES', () => {
	test('ids are unique and every theme defines every token as #rrggbb', () => {
		expect(new Set(THEMES.map((theme) => theme.id)).size).toBe(THEMES.length);
		for (const theme of THEMES) {
			expect(Object.keys(theme.colors).sort()).toEqual([...TOKENS].sort());
			for (const value of Object.values(theme.colors)) expect(value).toMatch(/^#[0-9a-f]{6}$/);
		}
	});

	// 下限取原来 ExHentai 和 E-Hentai 两套能达到的值，新加的配色不能比它们更难看清。
	test.each(THEMES.map((theme) => [theme.id, theme.colors] as const))('%s is readable', (_, c) => {
		for (const background of [c.bg, c.surface, c.raised, c.chip]) {
			expect(contrast(c.fg, background)).toBeGreaterThanOrEqual(4.5);
		}
		expect(contrast(c.muted, c.bg)).toBeGreaterThanOrEqual(3.5);
		for (const color of [c.link, c.danger, c.new, c.artist, c.group, c.parody]) {
			expect(contrast(color, c.bg)).toBeGreaterThanOrEqual(3);
		}
		expect(contrast(c['on-accent'], c.accent)).toBeGreaterThanOrEqual(3);
		expect(contrast(c['on-accent'], c.new)).toBeGreaterThanOrEqual(2.5);
		expect(contrast(c['on-accent'], c.danger)).toBeGreaterThanOrEqual(2.5);
	});
});

describe('activeThemes', () => {
	test('system uses the chosen dark and light themes', () => {
		const pair = activeThemes('system', 'catppuccin-mocha', 'tokyonight-day');
		expect(pair.dark.id).toBe('catppuccin-mocha');
		expect(pair.light.id).toBe('tokyonight-day');
	});

	test('a fixed theme is used for both schemes', () => {
		const pair = activeThemes('graphite', DEFAULT_DARK, DEFAULT_LIGHT);
		expect(pair.dark.id).toBe('graphite');
		expect(pair.light.id).toBe('graphite');
	});

	test('unknown ids and themes of the wrong scheme fall back to the defaults', () => {
		const pair = activeThemes('system', 'catppuccin-latte', 'removed');
		expect(pair.dark.id).toBe(DEFAULT_DARK);
		expect(pair.light.id).toBe(DEFAULT_LIGHT);
		expect(activeThemes('removed', DEFAULT_DARK, DEFAULT_LIGHT).dark.id).toBe(DEFAULT_DARK);
	});
});

test('themeGroups keeps the table order and filters by scheme', () => {
	expect(themeGroups().map((group) => group.family)).toEqual(['', 'Tokyo Night', 'Catppuccin']);
	const light = themeGroups('light').flatMap((group) => group.themes);
	expect(light.map((theme) => theme.id)).toEqual(['ehentai', 'tokyonight-day', 'catppuccin-latte']);
	expect(findTheme('ehentai', 'dark')).toBeUndefined();
});

/** WCAG 2 的对比度，测试里检查各套配色的文字是否看得清。 */
function contrast(a: string, b: string): number {
	const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p);
	return (x + 0.05) / (y + 0.05);
}

function luminance(hex: string): number {
	const channels = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
	const [r, g, b] = channels.map((c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
	return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
