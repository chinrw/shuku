import { describe, expect, it } from 'vitest';
import { namespaceClass, orderGroups } from './tags';

describe('orderGroups', () => {
	it('puts artist, group and parody first, then the rest by name, then bare tags', () => {
		const groups = ['other', 'language', '', 'parody', 'artist', 'female', 'group'].map(
			(namespace) => ({ namespace, values: ['x'] })
		);
		expect(orderGroups(groups).map((group) => group.namespace)).toEqual([
			'artist',
			'group',
			'parody',
			'female',
			'language',
			'other',
			''
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
