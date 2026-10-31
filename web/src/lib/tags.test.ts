import { describe, expect, it } from 'vitest';
import { byline, namespaceClass, parseTagText, rowTags, tagLine, tagRows, tagText } from './tags';

const item = {
	date_added: 1_700_000_000,
	source_url: 'https://example.com/g/1',
	tags: [
		{ namespace: '', values: ['独立标签'] },
		{ namespace: 'uploader', values: ['someone'] },
		{ namespace: 'group', values: ['サークル桜'] },
		{ namespace: 'other', values: ['full color'] },
		{ namespace: 'artist', values: ['三日月'] },
		{ namespace: 'timestamp', values: ['1600000000'] }
	]
};

describe('tagRows', () => {
	const rows = tagRows(item);

	it('sorts namespaces by name and merges bare tags into other, like LRR', () => {
		expect(rows.map((row) => row.label)).toEqual([
			'Artist',
			'Date Added',
			'Group',
			'Other',
			'Source',
			'Timestamp',
			'Uploader'
		]);
		expect(rows.find((row) => row.label === 'Other')?.values.map((value) => value.text)).toEqual([
			'独立标签',
			'full color'
		]);
	});

	it('links tags to an exact search and keeps the bare tag unprefixed', () => {
		const other = rows.find((row) => row.label === 'Other')!;
		expect(other.values[0].href).toBe(`/?q=${encodeURIComponent('"独立标签"')}`);
		expect(other.values[1].href).toBe(`/?q=${encodeURIComponent('"other:full color"')}`);
	});

	it('shows times as dates and the source as an external link', () => {
		const date = new Date(1_700_000_000 * 1000).toLocaleDateString();
		expect(rows.find((row) => row.label === 'Date Added')?.values).toEqual([
			{ text: date, href: null, external: false }
		]);
		expect(rows.find((row) => row.label === 'Timestamp')?.values[0].text).toBe(
			new Date(1_600_000_000 * 1000).toLocaleDateString()
		);
		expect(rows.find((row) => row.label === 'Source')?.values).toEqual([
			{ text: 'https://example.com/g/1', href: 'https://example.com/g/1', external: true }
		]);
	});

	it('adds https to a scheme-less source and drops unsafe schemes', () => {
		const rowsWith = (source_url: string) =>
			tagRows({ ...item, tags: [], source_url }).find((row) => row.label === 'Source')!.values[0];
		expect(rowsWith('example.com/g/2').href).toBe('https://example.com/g/2');
		expect(rowsWith('javascript:alert(1)').href).toBeNull();
	});
});

describe('tagLine', () => {
	it('lists values by namespace name and skips time namespaces', () => {
		expect(tagLine(item.tags).map((tag) => tag.text)).toEqual([
			'三日月',
			'サークル桜',
			'独立标签',
			'full color',
			'someone'
		]);
	});
});

describe('namespaceClass', () => {
	it('uses the LRR colors for artist, group or circle, and parody or series', () => {
		expect(namespaceClass('Artist')).toBe('ns-artist');
		expect(namespaceClass('circle')).toBe('ns-group');
		expect(namespaceClass('series')).toBe('ns-parody');
		expect(namespaceClass('female')).toBe('');
	});
});

describe('tag text', () => {
	it('writes namespaced tags the way LRR does', () => {
		expect(
			tagText([
				{ namespace: '', values: ['独立标签'] },
				{ namespace: 'artist', values: ['a', 'b'] }
			])
		).toBe('独立标签, artist:a, artist:b');
	});

	it('splits only on commas and drops empty items', () => {
		expect(parseTagText(' artist:a ,\nlanguage:中文,, title:你好，世界 ')).toEqual([
			'artist:a',
			'language:中文',
			'title:你好，世界'
		]);
		expect(parseTagText('  ')).toEqual([]);
		// 换行不是分隔符，原样交给服务端，由它拒绝。
		expect(parseTagText('artist:a\nb, c')).toEqual(['artist:a\nb', 'c']);
	});
});

describe('byline and rowTags', () => {
	const groups = [
		{ namespace: 'group', values: ['サークル桜'] },
		{ namespace: '', values: ['独立'] },
		{ namespace: 'artist', values: ['三日月', '鈴木'] },
		{ namespace: 'parody', values: ['original'] },
		{ namespace: 'date_added', values: ['1700000000'] }
	];

	it('puts artists before groups', () => {
		expect(byline(groups)).toBe('三日月 · 鈴木 · サークル桜');
		expect(byline([])).toBe('');
	});

	it('leaves artists, groups and dates out of the tag column', () => {
		expect(rowTags(groups, 5).map((tag) => tag.text)).toEqual(['独立', 'original']);
		expect(rowTags(groups, 1)).toHaveLength(1);
	});
});
