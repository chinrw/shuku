import { error } from '@sveltejs/kit';
import { ApiError } from './api';

export function parseId(raw: string): number {
	const id = Number(raw);
	if (!Number.isSafeInteger(id)) error(404, '没有这本书');
	return id;
}

/** 把接口的 404 转成 SvelteKit 的错误页，其他错误原样抛出。 */
export async function orNotFound<T>(request: Promise<T>): Promise<T> {
	try {
		return await request;
	} catch (cause) {
		if (cause instanceof ApiError && cause.status === 404) error(404, '没有这本书');
		throw cause;
	}
}
