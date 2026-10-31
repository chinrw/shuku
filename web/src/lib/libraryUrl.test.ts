import { describe, expect, it } from 'vitest';
import {
	libraryHref,
	parseCategory,
	parseFilter,
	parseOrder,
	parsePage,
	parseSort,
	parseTags
} from './libraryUrl';

describe('libraryHref', () => {
	it('always writes the sort, omits other defaults and keeps the rest', () => {
		expect(libraryHref({ q: '', sort: 'title', category: null })).toBe('/?sort=title');
		expect(libraryHref({ q: 'artist:a', sort: 'date', category: 3 })).toBe(
			'/?q=artist%3Aa&sort=date&c=3'
		);
		expect(libraryHref({ q: '', sort: 'title', category: 7, page: 1 })).toBe('/?sort=title&c=7');
		expect(libraryHref({ q: '', sort: 'title', category: null, page: 3 })).toBe(
			'/?sort=title&p=3'
		);
	});

	it('reads page numbers', () => {
		expect(parsePage('3')).toBe(3);
		expect(parsePage(null)).toBe(1);
		expect(parsePage('0')).toBe(1);
		expect(parsePage('2.5')).toBe(1);
		expect(parsePage(String(Number.MAX_SAFE_INTEGER))).toBe(1);
	});
});

describe('parseCategory', () => {
	it('accepts positive integers only', () => {
		expect(parseCategory('12')).toBe(12);
		expect(parseCategory(null)).toBeNull();
		expect(parseCategory('')).toBeNull();
		expect(parseCategory('0')).toBeNull();
		expect(parseCategory('1.5')).toBeNull();
		expect(parseCategory('abc')).toBeNull();
	});
});

describe('parseFilter', () => {
	it('accepts the known filters and ignores anything else', () => {
		expect(parseFilter('reading')).toBe('reading');
		expect(parseFilter('untagged')).toBe('untagged');
		expect(parseFilter(null)).toBeNull();
		expect(parseFilter('Reading')).toBeNull();
		expect(parseFilter('bogus')).toBeNull();
		expect(parseFilter('toString')).toBeNull();
	});

	it('round-trips through libraryHref', () => {
		expect(libraryHref({ q: '', sort: 'title', category: null, filter: 'new' })).toBe(
			'/?sort=title&f=new'
		);
		expect(libraryHref({ q: 'a', sort: 'title', category: 2, filter: null })).toBe(
			'/?q=a&sort=title&c=2'
		);
	});
});

describe('sort and order', () => {
	it('keeps known sorts and any ns: namespace', () => {
		expect(parseSort('pages')).toBe('pages');
		expect(parseSort('ns:artist')).toBe('ns:artist');
		expect(parseSort('title')).toBe('title');
		expect(parseSort('ns:')).toBeNull();
		expect(parseSort('size')).toBeNull();
		expect(parseSort(null)).toBeNull();
		expect(parseOrder('desc')).toBe('desc');
		expect(parseOrder('down')).toBeNull();
	});

	it('writes the order only when it differs from the default', () => {
		expect(libraryHref({ q: '', sort: 'date', order: 'desc', category: null })).toBe('/?sort=date');
		expect(libraryHref({ q: '', sort: 'date', order: 'asc', category: null })).toBe(
			'/?sort=date&o=asc'
		);
		expect(libraryHref({ q: '', sort: 'ns:artist', order: 'desc', category: null })).toBe(
			'/?sort=ns%3Aartist&o=desc'
		);
	});
});

describe('saved tag selection', () => {
	it('keeps the selected tags in the address', () => {
		expect(
			libraryHref({ q: '', sort: 'title', category: null, tags: ['artist:a', 'long hair'] })
		).toBe('/?sort=title&t=artist%3Aa&t=long+hair');
		expect(parseTags([' Artist:A ', '', 'artist:a', 'x'])).toEqual(['artist:a', 'x']);
	});
});
