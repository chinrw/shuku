import { describe, expect, it } from 'vitest';
import { bytes, clock, count, duration, percent } from './format';

describe('bytes', () => {
	it('keeps three significant digits in binary units', () => {
		expect(bytes(0)).toBe('0 B');
		expect(bytes(1023)).toBe('1023 B');
		expect(bytes(1536)).toBe('1.50 KiB');
		expect(bytes(412 * 1024 ** 2)).toBe('412 MiB');
		expect(bytes(1.82 * 1024 ** 4)).toBe('1.82 TiB');
		expect(bytes(30.5 * 1024 ** 3)).toBe('30.5 GiB');
	});
});

describe('count', () => {
	it('groups thousands', () => {
		expect(count(58756)).toBe('58,756');
		expect(count(7)).toBe('7');
	});
});

describe('duration', () => {
	it('shows the two largest units', () => {
		expect(duration(4_000)).toBe('4 秒');
		expect(duration(161_000)).toBe('2 分 41 秒');
		expect(duration(5_760_000)).toBe('1 小时 36 分');
		expect(duration(-5)).toBe('0 秒');
	});
});

describe('clock', () => {
	const now = new Date(2026, 9, 31, 15, 0, 0).getTime();

	it('shows only the time for today and adds the date otherwise', () => {
		expect(clock(new Date(2026, 9, 31, 14, 32, 10).getTime(), now)).toBe('14:32:10');
		expect(clock(new Date(2026, 9, 30, 9, 5, 0).getTime(), now)).toBe('10/30 09:05:00');
	});
});

describe('percent', () => {
	it('rounds and clamps, treating an empty total as zero', () => {
		expect(percent(1, 3)).toBe(33);
		expect(percent(5, 4)).toBe(100);
		expect(percent(0, 0)).toBe(0);
	});
});
