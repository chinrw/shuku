import { describe, expect, it } from 'vitest';
import { ago } from './time';

describe('ago', () => {
	const now = new Date(2026, 9, 31, 12, 0, 0).getTime();

	it('uses the largest whole unit under a month', () => {
		expect(ago(now - 30_000, now)).toBe('刚刚');
		expect(ago(now - 5 * 60_000, now)).toBe('5 分钟前');
		expect(ago(now - 3 * 3_600_000 - 1, now)).toBe('3 小时前');
		expect(ago(now - 2 * 86_400_000, now)).toBe('2 天前');
	});

	it('writes the date after thirty days', () => {
		expect(ago(new Date(2026, 0, 5, 8).getTime(), now)).toBe('2026/1/5');
	});

	it('treats times from a clock ahead of ours as just now', () => {
		expect(ago(now + 60_000, now)).toBe('刚刚');
	});
});
