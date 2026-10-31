import { describe, expect, it } from 'vitest';
import { libraryHref, parseCategory, parseFilter } from './libraryUrl';

describe('libraryHref', () => {
	it('omits defaults and keeps the rest', () => {
		expect(libraryHref({ q: '', sort: 'title', category: null })).toBe('/');
		expect(libraryHref({ q: 'artist:a', sort: 'date', category: 3 })).toBe(
			'/?q=artist%3Aa&sort=date&c=3'
		);
		expect(libraryHref({ q: '', sort: 'title', category: 7 })).toBe('/?c=7');
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
		expect(parseFilter(null)).toBeNull();
		expect(parseFilter('Reading')).toBeNull();
		expect(parseFilter('bogus')).toBeNull();
	});

	it('round-trips through libraryHref', () => {
		expect(libraryHref({ q: '', sort: 'title', category: null, filter: 'new' })).toBe('/?f=new');
		expect(libraryHref({ q: 'a', sort: 'title', category: 2, filter: null })).toBe('/?q=a&c=2');
	});
});
