import { describe, expect, it } from 'vitest';
import { tagTerm } from './query';

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
