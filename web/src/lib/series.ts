import type { ArchiveItem } from './api';

/**
 * 系列页“继续阅读”打开的那一卷：最近读过的一卷没读完就接着读它，读完了就读下一卷；
 * 一卷都没读过时从第一卷开始。最后一卷也读完了时还是最后一卷。
 */
export function nextVolume(volumes: ArchiveItem[]): ArchiveItem | undefined {
	let latest = -1;
	volumes.forEach((volume, index) => {
		const time = volume.progress?.client_time;
		if (time !== undefined && (latest < 0 || time > volumes[latest].progress!.client_time)) latest = index;
	});
	if (latest < 0) return volumes[0];
	const volume = volumes[latest];
	const finished = volume.progress!.page + 1 >= volume.page_count;
	return finished ? (volumes[latest + 1] ?? volume) : volume;
}
