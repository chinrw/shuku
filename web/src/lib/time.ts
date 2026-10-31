const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** 把 Unix 毫秒写成“3 小时前”这样的相对时间，超过 30 天写日期。 */
export function ago(time: number, now = Date.now()): string {
	const elapsed = now - time;
	if (elapsed < MINUTE) return '刚刚';
	if (elapsed < HOUR) return `${Math.floor(elapsed / MINUTE)} 分钟前`;
	if (elapsed < DAY) return `${Math.floor(elapsed / HOUR)} 小时前`;
	if (elapsed < 30 * DAY) return `${Math.floor(elapsed / DAY)} 天前`;
	const date = new Date(time);
	return `${date.getFullYear()}/${date.getMonth() + 1}/${date.getDate()}`;
}
