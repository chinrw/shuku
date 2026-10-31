import { describe, expect, it } from 'vitest';
import { artistQueries, relatedQueries } from './related';

describe('relatedQueries', () => {
	it('uses artists, groups and parodies in that order and skips original works', () => {
		const groups = [
			{ namespace: 'parody', values: ['original', 'blue archive'] },
			{ namespace: 'female', values: ['glasses'] },
			{ namespace: 'group', values: ['月光堂'] },
			{ namespace: 'artist', values: ['三日月'] }
		];
		expect(relatedQueries(groups)).toEqual([
			{ label: '作者：三日月', term: '"artist:三日月"' },
			{ label: '社团：月光堂', term: '"group:月光堂"' },
			{ label: '原作：blue archive', term: '"parody:blue archive"' }
		]);
	});

	it('lists series first and uses publishers for volume-based manga', () => {
		const groups = [
			{ namespace: 'publisher', values: ['集英社'] },
			{ namespace: 'artist', values: ['尾田栄一郎'] },
			{ namespace: 'series', values: ['ONE PIECE'] }
		];
		expect(relatedQueries(groups)).toEqual([
			{ label: '系列：ONE PIECE', term: '"series:ONE PIECE"' },
			{ label: '作者：尾田栄一郎', term: '"artist:尾田栄一郎"' },
			{ label: '出版社：集英社', term: '"publisher:集英社"' }
		]);
	});

	it('stops at four rows', () => {
		const groups = [{ namespace: 'artist', values: ['a', 'b', 'c', 'd', 'e'] }];
		expect(relatedQueries(groups)).toHaveLength(4);
	});
});

describe('artistQueries', () => {
	it('lists only artists and stops at two rows', () => {
		const groups = [
			{ namespace: 'series', values: ['ONE PIECE'] },
			{ namespace: 'Artist', values: ['a', 'b', 'c'] }
		];
		expect(artistQueries(groups)).toEqual([
			{ label: '作者：a', term: '"Artist:a"' },
			{ label: '作者：b', term: '"Artist:b"' }
		]);
	});

	it('reads the Chinese namespaces of translated tags', () => {
		const groups = [
			{ namespace: '原作', values: ['原创'] },
			{ namespace: '艺术家', values: ['三日月'] },
			{ namespace: '团队', values: ['月光堂'] }
		];
		expect(artistQueries(groups)).toEqual([{ label: '作者：三日月', term: '"艺术家:三日月"' }]);
		expect(relatedQueries(groups)).toEqual([
			{ label: '作者：三日月', term: '"艺术家:三日月"' },
			{ label: '社团：月光堂', term: '"团队:月光堂"' }
		]);
	});
});
