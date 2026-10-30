import { describe, expect, it } from 'vitest';
import { relatedQueries } from './related';

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

	it('stops at four rows', () => {
		const groups = [{ namespace: 'artist', values: ['a', 'b', 'c', 'd', 'e'] }];
		expect(relatedQueries(groups)).toHaveLength(4);
	});
});
