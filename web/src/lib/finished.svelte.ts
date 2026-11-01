import { SvelteMap } from 'svelte/reactivity';
import { setFinished, type Progress } from './api';

// 这次打开网页以来手动改过、或者阅读器上报到最后一页的“读完”标记，都是服务端已经
// 写进去的结果。书库的列表来自搜索快照，约 10 秒后才包含它们，这段时间里角标和按钮
// 按这里的显示；之后重新取到的列表或详情以服务端为准，别的设备后来的改动不会被盖住。
// 服务端的快照刷新延迟（10 秒）加一点余量，和书库页重新加载的延迟一致。
const SNAPSHOT_LAG = 11_000;
const overrides = new SvelteMap<number, { finished: boolean; at: number }>();

/** 这本书现在算不算读完：刚改过的看这里，否则看服务端给的进度。 */
export function finishedOf(id: number, progress: Progress | null | undefined): boolean {
	return overrides.get(id)?.finished ?? progress?.finished ?? false;
}

/** 记下一批书已经写进服务端的新标记。 */
export function noteFinished(ids: number[], finished: boolean) {
	const at = Date.now();
	for (const id of ids) overrides.set(id, { finished, at });
}

/** 换上了 `now` 时从搜索快照取的列表：快照已经包含的改动不用再记着。 */
export function settleFinished(now: number) {
	for (const [id, { at }] of overrides) {
		if (now - at >= SNAPSHOT_LAG) overrides.delete(id);
	}
}

/** 按条件批量改了标记，不知道是哪几本：全部丢掉，等快照刷新。 */
export function forgetFinished() {
	overrides.clear();
}

/** 标记读完或取消，按服务端的回复记下，并返回服务端现在的进度。 */
export async function markFinished(id: number, finished: boolean): Promise<Progress | null> {
	const progress = await setFinished(id, finished);
	noteFinished([id], progress?.finished ?? false);
	return progress;
}
