import { describe, expect, it } from 'vitest';
import { BoundedCache } from './bounded';

/** 只实现 BoundedCache 用到的部分，按存入顺序保存，和 Cache Storage 一样。 */
class FakeCache {
	entries = new Map<string, Response>();

	async put(url: string, response: Response) {
		this.entries.delete(url);
		this.entries.set(url, response);
	}
	async match(key: string | Request) {
		return this.entries.get(typeof key === 'string' ? key : key.url)?.clone();
	}
	async keys() {
		return [...this.entries.keys()].map((url) => new Request(url));
	}
	async delete(key: Request) {
		return this.entries.delete(key.url);
	}
}

const image = (size: number, sized = true) =>
	new Response(new Uint8Array(size), {
		headers: sized ? { 'content-length': String(size) } : {}
	});
const url = (n: number) => `https://shuku.test/api/archives/1/pages/${n}?v=h`;

function setup(budget: number) {
	let fake = new FakeCache();
	const storage = {
		open: async () => fake as unknown as Cache,
		delete: async () => {
			fake = new FakeCache();
			return true;
		}
	};
	const cache = new BoundedCache('pages', async () => budget, storage);
	return { fake: () => fake, cache };
}

describe('BoundedCache', () => {
	it('evicts the oldest entries once the budget is exceeded, down to 90%', async () => {
		const { fake: current, cache } = setup(100);
		const fake = current();
		for (let n = 0; n < 5; n++) await cache.put(url(n), image(20));
		expect(fake.entries.size).toBe(5);
		await cache.put(url(5), image(20));
		// 120 > 100，删到 90 以下：去掉最早的两条。
		expect([...fake.entries.keys()]).toEqual([url(2), url(3), url(4), url(5)]);
	});

	it('moves a re-stored URL to the end so it is evicted last', async () => {
		const { fake: current, cache } = setup(100);
		const fake = current();
		for (let n = 0; n < 5; n++) await cache.put(url(n), image(20));
		await cache.put(url(0), image(20));
		await cache.put(url(5), image(20));
		expect([...fake.entries.keys()]).toEqual([url(3), url(4), url(0), url(5)]);
	});

	it('skips a single response larger than a quarter of the budget', async () => {
		const { fake: current, cache } = setup(100);
		const fake = current();
		await cache.put(url(0), image(26));
		expect(fake.entries.size).toBe(0);
	});

	it('measures responses without Content-Length and records the size', async () => {
		const { fake: current, cache } = setup(100);
		const fake = current();
		await cache.put(url(0), image(10, false));
		const stored = await fake.match(url(0));
		expect(stored?.headers.get('content-length')).toBe('10');
		expect((await stored!.arrayBuffer()).byteLength).toBe(10);
	});

	it('keeps the count right when the same URL is replaced concurrently', async () => {
		const { fake: current, cache } = setup(100);
		const fake = current();
		for (let n = 0; n < 5; n++) await cache.put(url(n), image(20));
		await Promise.all([cache.put(url(0), image(1)), cache.put(url(0), image(1))]);
		// 实际 81，再存 20 超出上限，删掉最早的一条。两次替换都按 20 扣减的话计数是 62，不会淘汰。
		await cache.put(url(5), image(20));
		expect([...fake.entries.keys()]).toEqual([url(2), url(3), url(4), url(0), url(5)]);
	});

	it('drops writes that were not current when their turn came', async () => {
		const { fake, cache } = setup(100);
		await cache.put(url(0), image(10), () => false);
		expect(fake().entries.size).toBe(0);
	});

	it('clears after pending writes and starts counting afresh', async () => {
		const { fake, cache } = setup(100);
		for (let n = 0; n < 4; n++) await cache.put(url(n), image(20));
		// 退出登录前发出的写入：清掉时它已经不算数了。
		let current = true;
		const pending = cache.put(url(4), image(20), () => current);
		current = false;
		await cache.clear();
		await pending;
		expect(fake().entries.size).toBe(0);
		// 清掉前计数是 100；清掉后从 0 数起，再存 5 条不该淘汰。
		for (let n = 0; n < 5; n++) await cache.put(url(n), image(20));
		expect(fake().entries.size).toBe(5);
	});

	it('counts entries stored before the worker restarted', async () => {
		const { fake: current, cache } = setup(100);
		const fake = current();
		for (let n = 0; n < 4; n++) await fake.put(url(n), image(20));
		await cache.put(url(4), image(20));
		expect(fake.entries.size).toBe(5);
		await cache.put(url(5), image(20));
		expect(fake.entries.size).toBe(4);
	});
});
