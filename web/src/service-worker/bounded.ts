// 有容量上限的 Cache Storage。一张原图最大能到 25 MB，不限的话读几本书就能占掉几个 GB。

/** 删到上限的这个比例以下再停，免得每存一张就删一张。 */
const TRIM_TARGET = 0.9;
/** 单个响应超过上限的这个比例就不存，免得一张大图把其他缓存全挤掉。 */
const MAX_ENTRY = 0.25;

/**
 * 按存入的先后淘汰：Cache Storage 的 `keys()` 按存入顺序返回，`put` 同一个地址会移到最后。
 * 命中缓存时不调整顺序（那要把整张图再写一遍），所以是先进先出，近似“最近读的书留着”。
 */
export class BoundedCache {
	/** 已存的字节数。第一次存入时数一遍，之后累加；删除时重新数，顺带修正累加的误差。 */
	#total: number | null = null;
	#queue: Promise<void> = Promise.resolve();

	constructor(
		private readonly name: string,
		/** 上限可能要问 `navigator.storage.estimate()`，所以是异步的。 */
		private readonly budget: () => Promise<number>,
		private readonly storage: Pick<CacheStorage, 'open' | 'delete'> = caches
	) {}

	async match(url: string): Promise<Response | undefined> {
		return (await this.storage.open(this.name)).match(url);
	}

	/**
	 * 存入一个响应。超过上限时从最早存入的开始删。轮到写入时 `current()` 为假就不写，
	 * 用来丢掉退出登录之前发出、之后才返回的请求。
	 */
	async put(url: string, response: Response, current: () => boolean = () => true): Promise<void> {
		const budget = await this.budget();
		let size = Number(response.headers.get('content-length'));
		if (!Number.isFinite(size) || size <= 0) {
			// 服务端的图片和 JSON 都带 Content-Length，没有时才读完整个响应来数。
			const body = await response.blob();
			size = body.size;
			const headers = new Headers(response.headers);
			headers.set('content-length', String(size));
			response = new Response(body, {
				status: response.status,
				statusText: response.statusText,
				headers
			});
		}
		if (size > budget * MAX_ENTRY) return;
		// 存入、计数和删除整个排队：并发替换同一个地址时，各自读到的旧大小才不会重复扣减。
		// 阅读器一次最多预取两张图，除了动图，同时取两张时都是一两 MB 的缩小的图，排队不会
		// 让多少响应积在内存里。
		return this.#enqueue(() => (current() ? this.#store(url, response, size, budget) : undefined));
	}

	/** 删掉整个缓存。排在已经开始的写入后面，所以删完以后不会再有旧的写入。 */
	clear(): Promise<void> {
		return this.#enqueue(async () => {
			await this.storage.delete(this.name);
			this.#total = null;
		});
	}

	#enqueue(task: () => Promise<void> | undefined): Promise<void> {
		const step = this.#queue.then(task);
		this.#queue = step.catch(() => {});
		return step;
	}

	async #store(url: string, response: Response, size: number, budget: number) {
		const cache = await this.storage.open(this.name);
		// 同一个地址再存一次（接口的 JSON）会替换旧的，旧的大小要减掉。
		const replaced = await entrySize(cache, url);
		await cache.put(url, response);
		// 第一次数时已经包含了刚存入的这一条。
		this.#total = this.#total === null ? await sizeOf(cache) : this.#total + size - replaced;
		if (this.#total > budget) this.#total = await trim(cache, budget * TRIM_TARGET);
	}
}

async function entrySize(cache: Cache, key: Request | string): Promise<number> {
	const response = await cache.match(key);
	return Number(response?.headers.get('content-length')) || 0;
}

async function sizeOf(cache: Cache): Promise<number> {
	let total = 0;
	for (const key of await cache.keys()) total += await entrySize(cache, key);
	return total;
}

/** 从最早存入的开始删，直到不超过 `target`。返回剩下的字节数。 */
async function trim(cache: Cache, target: number): Promise<number> {
	const keys = await cache.keys();
	const sizes = await Promise.all(keys.map((key) => entrySize(cache, key)));
	let total = sizes.reduce((sum, size) => sum + size, 0);
	for (let i = 0; i < keys.length && total > target; i++) {
		await cache.delete(keys[i]);
		total -= sizes[i];
	}
	return total;
}
