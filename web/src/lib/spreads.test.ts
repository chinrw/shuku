import { describe, expect, it } from 'vitest';
import { buildSpreads } from './spreads';

const tall = { width: 1200, height: 1700 };
const wide = { width: 2400, height: 1700 };

describe('buildSpreads', () => {
	it('shows the cover alone and pairs the following pages', () => {
		expect(buildSpreads([tall, tall, tall, tall, tall])).toEqual([[0], [1, 2], [3, 4]]);
	});

	it('leaves a trailing odd page alone', () => {
		expect(buildSpreads([tall, tall, tall, tall])).toEqual([[0], [1, 2], [3]]);
	});

	it('shows wide pages alone because they already are spreads', () => {
		expect(buildSpreads([tall, tall, wide, tall, tall])).toEqual([[0], [1], [2], [3, 4]]);
	});

	it('handles books with no pages', () => {
		expect(buildSpreads([])).toEqual([]);
	});
});
