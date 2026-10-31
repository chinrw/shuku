import { status, type Status } from './admin';

/** 控制台打开时每隔这么久刷新一次状态。 */
const INTERVAL = 5000;

/**
 * 控制台各页共用的状态。布局挂载时开始轮询，离开控制台后停止；
 * 操作完成后调用 refresh 立即更新，不等下一轮。
 */
class AdminStatus {
	current = $state<Status | null>(null);
	error = $state<string | null>(null);
	/** 最近一次成功取到状态的时间，Unix 毫秒。 */
	updated = $state(0);
	#timer: ReturnType<typeof setInterval> | undefined;

	async refresh() {
		try {
			this.current = await status();
			this.error = null;
			this.updated = Date.now();
		} catch (error) {
			this.error = error instanceof Error ? error.message : String(error);
		}
	}

	/** 返回停止轮询的函数，给 $effect 当清理函数用。 */
	poll(): () => void {
		this.refresh();
		this.#timer = setInterval(() => this.refresh(), INTERVAL);
		return () => clearInterval(this.#timer);
	}
}

export const adminStatus = new AdminStatus();
