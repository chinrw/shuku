import { describe, expect, it } from 'vitest';
import { finishedOf, forgetFinished, noteFinished, settleFinished } from './finished.svelte';

const done = { page: 9, client_time: 1, finished: true };

describe('finished overrides', () => {
	it('shows a fresh change until the snapshot has caught up', () => {
		forgetFinished();
		noteFinished([1], false);
		expect(finishedOf(1, done)).toBe(false);
		settleFinished(Date.now() + 1_000);
		expect(finishedOf(1, done)).toBe(false);
		settleFinished(Date.now() + 12_000);
		expect(finishedOf(1, done)).toBe(true);
	});

	it('drops everything after a batch over search results', () => {
		noteFinished([2], true);
		forgetFinished();
		expect(finishedOf(2, null)).toBe(false);
	});
});
