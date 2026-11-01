import { describe, expect, it } from 'vitest';
import { isPageSize, pageCount, pageNumbers } from './pages';

describe('pageNumbers', () => {
	it('shows every page when there are few', () => {
		expect(pageNumbers(1, 1)).toEqual([1]);
		expect(pageNumbers(2, 5)).toEqual([1, 2, 3, 4, 5]);
	});

	it('keeps the ends and two pages around the current one', () => {
		expect(pageNumbers(10, 20)).toEqual([1, null, 8, 9, 10, 11, 12, null, 20]);
		expect(pageNumbers(1, 20)).toEqual([1, 2, 3, null, 20]);
		expect(pageNumbers(20, 20)).toEqual([1, null, 18, 19, 20]);
	});

	it('treats a page past the end as the last page', () => {
		expect(pageNumbers(Number.MAX_SAFE_INTEGER, 20)).toEqual([1, null, 18, 19, 20]);
	});

	it('fills a one-page gap instead of an ellipsis', () => {
		expect(pageNumbers(5, 20)).toEqual([1, 2, 3, 4, 5, 6, 7, null, 20]);
	});
});

describe('pageCount', () => {
	it('rounds up and is at least one', () => {
		expect(pageCount(0, 100)).toBe(1);
		expect(pageCount(100, 100)).toBe(1);
		expect(pageCount(101, 100)).toBe(2);
		expect(pageCount(101, 50)).toBe(3);
	});
});

describe('isPageSize', () => {
	it('accepts only the offered sizes', () => {
		expect(isPageSize(50)).toBe(true);
		expect(isPageSize(200)).toBe(true);
		expect(isPageSize(150)).toBe(false);
		expect(isPageSize('100')).toBe(false);
	});
});
