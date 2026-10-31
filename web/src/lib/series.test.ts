import { describe, expect, it } from 'vitest';
import type { ArchiveItem } from './api';
import { nextVolume } from './series';

function volume(id: number, progress: [page: number, time: number] | null): ArchiveItem {
	return {
		id,
		title: `第${id}卷`,
		page_count: 10,
		is_new: false,
		progress: progress && { page: progress[0], client_time: progress[1] },
		cover_url: '',
		date_added: 0,
		source_url: null,
		tags: [],
		series: null
	};
}

describe('nextVolume', () => {
	it('starts from the first volume when none has been read', () => {
		expect(nextVolume([volume(1, null), volume(2, null)])?.id).toBe(1);
	});

	it('continues the most recently read volume until it is finished', () => {
		const volumes = [volume(1, [9, 100]), volume(2, [3, 200]), volume(3, null)];
		expect(nextVolume(volumes)?.id).toBe(2);
	});

	it('moves on to the next volume after finishing one', () => {
		const volumes = [volume(1, [9, 100]), volume(2, [9, 300]), volume(3, [2, 50])];
		expect(nextVolume(volumes)?.id).toBe(3);
	});

	it('stays on the last volume when the whole series is finished', () => {
		expect(nextVolume([volume(1, [9, 100]), volume(2, [9, 200])])?.id).toBe(2);
	});
});
