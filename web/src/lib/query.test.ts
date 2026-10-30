import { describe, expect, it } from 'vitest';
import { addTerm, completeTerm, lastTerm, tagTerm } from './query';

describe('tagTerm', () => {
	it('quotes namespaced tags for an exact match', () => {
		expect(tagTerm('artist', 'Alpha, Beta')).toBe('"artist:Alpha, Beta"');
	});

	it('uses the bare value when the tag has no namespace', () => {
		expect(tagTerm('', '独立')).toBe('"独立"');
	});

	it('falls back to the $ marker when the value contains a quote', () => {
		expect(tagTerm('title', 'say "hi"')).toBe('title:say "hi"$');
	});
});

describe('lastTerm', () => {
	it('takes the text after the last comma outside quotes', () => {
		expect(lastTerm('"artist:a, b", -fem')).toEqual({
			head: '"artist:a, b",',
			text: 'fem',
			excluded: true,
			done: false
		});
	});

	it('treats the whole input as one term when there is no comma', () => {
		expect(lastTerm('  "lang')).toEqual({ head: '', text: 'lang', excluded: false, done: false });
	});

	it('marks quoted and $-terminated terms as done', () => {
		expect(lastTerm('a, "artist:b"').done).toBe(true);
		expect(lastTerm('artist:b$').done).toBe(true);
		expect(lastTerm('"').done).toBe(false);
	});
});

describe('completeTerm', () => {
	it('replaces the last term and keeps the exclusion mark', () => {
		expect(completeTerm('"artist:a", -fem', 'female:glasses')).toBe(
			'"artist:a", -"female:glasses", '
		);
	});

	it('completes the first term', () => {
		expect(completeTerm('alp', 'artist:alpha')).toBe('"artist:alpha", ');
	});
});

describe('addTerm', () => {
	it('appends an exact tag term after a comma', () => {
		expect(addTerm('foo, ', 'artist:alpha')).toBe('foo, "artist:alpha"');
	});

	it('starts a new query when the input is empty', () => {
		expect(addTerm('  ', 'artist:alpha')).toBe('"artist:alpha"');
	});
});
