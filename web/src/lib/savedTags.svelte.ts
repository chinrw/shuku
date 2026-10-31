import { SvelteMap } from 'svelte/reactivity';
import { ApiError, savedTags as fetchSavedTags, setSavedTag } from './api';

// 收藏的标签。书库筛选行、标签弹窗、详情抽屉和阅读器的详情面板共用一份，在一处收藏，
// 筛选行马上多出这个标签。列表存在服务端，所有设备共用。
class SavedTags {
	/** 显示用的列表：服务端最后一次给的列表，加上还没提交完的收藏和取消。 */
	tags = $state<string[]>([]);
	error = $state('');
	/** 还没提交完的操作，标签 → 收藏还是取消。同一个标签同时只有一个。 */
	private ops = new SvelteMap<string, boolean>();
	private server: string[] = [];
	private loaded = false;
	/** 每次本地改动开始和写完各加一。读取开始后有过改动，读到的列表可能比写完的旧，就不用。 */
	private version = 0;
	// 写请求排队一个一个发：服务端每次返回整个列表，并发时晚到的旧列表会盖掉新的。
	private queue: Promise<void> = Promise.resolve();

	private refresh() {
		const kept = this.server.filter((tag) => this.ops.get(tag) !== false);
		const added = [...this.ops].filter(([tag, on]) => on && !kept.includes(tag)).map(([tag]) => tag);
		this.tags = [...kept, ...added];
	}

	/** 开始读取前取一个标记，读完交给 `set`。 */
	mark(): number {
		return this.version;
	}

	/** 书库页加载时取到的列表。读取期间本地改过的话丢掉它，以写请求返回的为准。 */
	set(tags: string[], mark: number) {
		if (mark !== this.version) return;
		this.server = tags;
		this.loaded = true;
		this.refresh();
	}

	/** 还没取过时取一次；书库页之外（阅读器、详情页）第一次显示收藏按钮时用。 */
	async ensure() {
		if (this.loaded) return;
		this.loaded = true;
		const mark = this.mark();
		try {
			this.set(await fetchSavedTags(fetch), mark);
		} catch (cause) {
			this.loaded = false;
			console.warn('加载收藏的标签失败', cause);
		}
	}

	has(tag: string): boolean {
		return this.tags.includes(tag);
	}

	isPending(tag: string): boolean {
		return this.ops.has(tag);
	}

	toggle(tag: string) {
		if (this.ops.has(tag)) return;
		const saved = !this.has(tag);
		this.ops.set(tag, saved);
		this.version++;
		this.error = '';
		this.refresh();
		this.queue = this.queue.then(async () => {
			try {
				this.server = await setSavedTag(tag, saved);
				this.loaded = true;
			} catch (cause) {
				this.error = cause instanceof ApiError ? cause.message : '网络错误';
				// 失败的写请求没带回列表；这时手里的列表可能还是空的（首次读取因为这次改动被丢掉），
				// 重新取一次服务端的。
				this.server = await fetchSavedTags(fetch).catch(() => this.server);
			} finally {
				// 失败时去掉这一项就是撤销：显示回到服务端的列表加上其余还没提交完的操作。
				this.ops.delete(tag);
				this.version++;
				this.refresh();
			}
		});
	}
}

let instance: SavedTags | undefined;

export function savedTagStore(): SavedTags {
	instance ??= new SavedTags();
	return instance;
}
