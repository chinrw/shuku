import { describe, expect, it } from 'vitest';
import type { LogEntry } from './admin';
import { LOG_LIMIT, appendLogs } from './logBuffer';

const entry = (seq: number): LogEntry => ({ seq, time: seq, level: 'info', message: `#${seq}` });

describe('appendLogs', () => {
	it('appends only entries newer than the last one shown', () => {
		const merged = appendLogs([entry(1), entry(2)], [entry(2), entry(3)]);
		expect(merged.map((item) => item.seq)).toEqual([1, 2, 3]);
	});

	it('drops the oldest entries past the limit', () => {
		const many = Array.from({ length: LOG_LIMIT }, (_, index) => entry(index + 1));
		const merged = appendLogs(many, [entry(LOG_LIMIT + 1)]);
		expect(merged).toHaveLength(LOG_LIMIT);
		expect(merged[0].seq).toBe(2);
		expect(merged.at(-1)?.seq).toBe(LOG_LIMIT + 1);
	});
});
