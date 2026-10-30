import { putProgress, type Progress } from './api';

type Send = (
	id: number,
	progress: Progress,
	options: { keepalive: boolean }
) => Promise<{ accepted: boolean; progress: Progress }>;

/**
 * 翻页时记下页码和翻页时刻，停顿一段时间后只上报最后一页。
 * 服务端按 client_time 判断新旧，所以时刻取翻页的那一刻，不取发送的时刻。
 */
export class ProgressReporter {
	#pending: Progress | null = null;
	#timer: ReturnType<typeof setTimeout> | undefined;

	constructor(
		private readonly id: number,
		/** 定时发送失败时调用；直接调用 flush 时错误由调用方处理。 */
		private readonly onError: (error: unknown) => void,
		private readonly send: Send = putProgress,
		private readonly delay = 1000
	) {}

	turn(page: number, now = Date.now()) {
		this.#pending = { page, client_time: now };
		clearTimeout(this.#timer);
		this.#timer = setTimeout(() => this.flush().catch(this.onError), this.delay);
	}

	/** 离开阅读页时用 keepalive 立即发送，页面卸载后请求仍会完成。 */
	async flush(keepalive = false) {
		clearTimeout(this.#timer);
		const progress = this.#pending;
		if (progress === null) return;
		this.#pending = null;
		await this.send(this.id, progress, { keepalive });
	}
}
