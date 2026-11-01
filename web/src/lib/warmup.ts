import { route } from '../service-worker/routes';

type Entry = { name: string; initiatorType: string; startTime: number };

/**
 * 在 `before`（Service Worker 接管的时刻）之前发出、所以没进它缓存的接口地址（去重）。
 * 只挑 Service Worker 会缓存的读接口，写接口和图片不重取。
 */
export function uncachedApiRequests(entries: readonly Entry[], origin: string, before: number): string[] {
	const urls = entries
		.filter((entry) => entry.initiatorType === 'fetch' && entry.startTime < before)
		.map((entry) => entry.name)
		.filter((url) => route({ method: 'GET', mode: 'cors', url }, origin) === 'api');
	return [...new Set(urls)];
}

/**
 * Service Worker 接管时调用。第一次打开时页面的数据在接管之前就取了，不经过它，也就没进缓存；
 * 把这些接口再取一次，经过 Service Worker 存进缓存，第一次打开的书断网后也能重开。
 * 接管时还没取完的请求不在资源记录里，等它们完成时再补。
 */
export function warmUp(origin: string) {
	const takeover = performance.now();
	const fetched = new Set<string>();
	const refetch = (entries: readonly Entry[]) => {
		for (const url of uncachedApiRequests(entries, origin, takeover)) {
			if (fetched.has(url)) continue;
			fetched.add(url);
			fetch(url).catch((error) => console.warn('接口没能存进离线缓存', url, error));
		}
	};
	refetch(performance.getEntriesByType('resource') as PerformanceResourceTiming[]);
	new PerformanceObserver((list) => refetch(list.getEntries() as PerformanceResourceTiming[])).observe({
		type: 'resource'
	});
}
