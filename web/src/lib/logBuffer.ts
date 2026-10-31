import type { LogEntry } from './admin';

/** 页面上最多留这么多条，与服务端的缓冲区一样大。 */
export const LOG_LIMIT = 1000;

/** 把新取到的日志接在后面，按序号去重，超出上限时丢掉最旧的。 */
export function appendLogs(current: LogEntry[], incoming: LogEntry[]): LogEntry[] {
	const last = current.at(-1)?.seq ?? 0;
	const merged = [...current, ...incoming.filter((entry) => entry.seq > last)];
	return merged.length > LOG_LIMIT ? merged.slice(merged.length - LOG_LIMIT) : merged;
}
