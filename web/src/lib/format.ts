const UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];

/** 1024 进制，保留三位有效数字：`412 MiB`、`1.82 TiB`。 */
export function bytes(value: number): string {
	let size = value;
	let unit = 0;
	while (size >= 1024 && unit < UNITS.length - 1) {
		size /= 1024;
		unit += 1;
	}
	if (unit === 0) return `${value} B`;
	const digits = size >= 100 ? 0 : size >= 10 ? 1 : 2;
	return `${size.toFixed(digits)} ${UNITS[unit]}`;
}

/** 千分位：`58,756`。 */
export function count(value: number): string {
	return value.toLocaleString('en-US');
}

/** 时长写成“1 小时 36 分”“2 分 41 秒”“5 秒”，只留最大的两级。 */
export function duration(ms: number): string {
	const seconds = Math.max(0, Math.round(ms / 1000));
	const hours = Math.floor(seconds / 3600);
	const minutes = Math.floor((seconds % 3600) / 60);
	const rest = seconds % 60;
	if (hours > 0) return `${hours} 小时 ${minutes} 分`;
	if (minutes > 0) return `${minutes} 分 ${rest} 秒`;
	return `${rest} 秒`;
}

/** 当天的时刻 `14:32:10`，不是当天的加上日期。 */
export function clock(time: number, now = Date.now()): string {
	const date = new Date(time);
	const pad = (n: number) => String(n).padStart(2, '0');
	const hms = `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
	return new Date(now).toDateString() === date.toDateString()
		? hms
		: `${date.getMonth() + 1}/${date.getDate()} ${hms}`;
}

/** 0 到 100 的百分比，总数为 0 时算 0。 */
export function percent(part: number, total: number): number {
	return total > 0 ? Math.min(100, Math.round((part / total) * 100)) : 0;
}
