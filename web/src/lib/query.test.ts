import { describe, expect, it } from 'vitest';
import {
	addTerm,
	completeTerm,
	completeWord,
	lastTerm,
	lastWord,
	stripTrailingSeparator,
	tagTerm
} from './query';

describe('tagTerm', () => {
	it('quotes namespaced tags for an exact match', () => {
		expect(tagTerm('artist', 'Alpha, Beta')).toBe('"artist:Alpha, Beta"');
	});

	it('uses the bare value when the tag has no namespace', () => {
		expect(tagTerm('', '独立')).toBe('"独立"');
	});

	it('escapes quotes inside the quoted term', () => {
		expect(tagTerm('title', 'say "hi"')).toBe('"title:say \\"hi\\""');
	});

	it('escapes wildcard characters and backslashes so the tag matches literally', () => {
		expect(tagTerm('artist', 'foo_bar')).toBe('"artist:foo\\_bar"');
		expect(tagTerm('', 'a*b%c?d\\e')).toBe('"a\\*b\\%c\\?d\\\\e"');
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

	it('treats escaped quotes, commas and markers as plain characters', () => {
		expect(lastTerm('"artist:a\\", b')).toEqual({
			head: '',
			text: 'artist:a", b',
			excluded: false,
			done: false
		});
		expect(lastTerm('a\\,b')).toEqual({ head: '', text: 'a,b', excluded: false, done: false });
		expect(lastTerm('"artist:foo\\_"').done).toBe(true);
		expect(lastTerm('"artist:foo\\_').text).toBe('artist:foo_');
		expect(lastTerm('artist:b\\$').done).toBe(false);
		expect(lastTerm('"artist:b\\"').done).toBe(false);
		expect(lastTerm('"artist:b\\\\"').done).toBe(true);
	});
});

describe('lastWord', () => {
	it('takes the word after the last space of the last term', () => {
		expect(lastWord('"artist:a", big -lo')).toEqual({
			before: '"artist:a", big',
			text: 'lo',
			excluded: true
		});
	});

	it('is null without a space, for quoted or finished terms and after a trailing space', () => {
		expect(lastWord('a, lo')).toBeNull();
		expect(lastWord('"long ha')).toBeNull();
		expect(lastWord('alpha "long ha')).toBeNull();
		expect(lastWord('- "long ha')).toBeNull();
		expect(lastWord('alpha lo$')).toBeNull();
		expect(lastWord('alpha lo ')).toBeNull();
	});
});

describe('completeWord', () => {
	it('turns the space before the word into a separator', () => {
		expect(completeWord('alpha lo', 'long hair')).toBe('alpha, "long hair", ');
		expect(completeWord('x, alpha -lo', 'long hair')).toBe('x, alpha, -"long hair", ');
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

	it('escapes the picked tag so the completed term is done and round-trips', () => {
		const completed = completeTerm('foo', 'artist:foo_bar');
		expect(completed).toBe('"artist:foo\\_bar", ');
		expect(lastTerm(completed.trimEnd().replace(/,$/, '')).done).toBe(true);
	});
});

describe('stripTrailingSeparator', () => {
	it('removes a real trailing separator', () => {
		expect(stripTrailingSeparator('a, b, ')).toBe('a, b');
	});

	it('keeps an escaped comma', () => {
		expect(stripTrailingSeparator('artist:foo\\,')).toBe('artist:foo\\,');
	});

	it('keeps a comma inside quotes', () => {
		expect(stripTrailingSeparator('"x, y",')).toBe('"x, y"');
		expect(stripTrailingSeparator('"artist:a,')).toBe('"artist:a,');
	});

	it('returns an empty string for blank input', () => {
		expect(stripTrailingSeparator('')).toBe('');
		expect(stripTrailingSeparator('  ')).toBe('');
	});
});

describe('addTerm', () => {
	it('appends an exact tag term after a comma', () => {
		expect(addTerm('foo, ', 'artist:alpha')).toBe('foo, "artist:alpha"');
	});

	it('starts a new query when the input is empty', () => {
		expect(addTerm('  ', 'artist:alpha')).toBe('"artist:alpha"');
	});

	it('doubles a lone trailing backslash so it cannot escape the new separator', () => {
		expect(addTerm('artist:foo\\', 'language:chinese')).toBe(
			'artist:foo\\\\, "language:chinese"'
		);
	});

	it('leaves an already escaped trailing backslash alone', () => {
		expect(addTerm('artist:foo\\\\', 'language:chinese')).toBe(
			'artist:foo\\\\, "language:chinese"'
		);
	});

	it('keeps an escaped trailing comma as part of the term', () => {
		expect(addTerm('artist:foo\\,', 'language:chinese')).toBe(
			'artist:foo\\,, "language:chinese"'
		);
	});
});
