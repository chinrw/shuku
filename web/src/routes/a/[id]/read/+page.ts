import { archive, pages } from '#lib/api.ts';
import { orNotFound, parseId } from '#lib/load.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params, url }) => {
	const id = parseId(params.id);
	const [book, list] = await orNotFound(Promise.all([archive(fetch, id), pages(fetch, id)]));
	const requested = Number(url.searchParams.get('page') ?? book.progress?.page ?? 0);
	const last = Math.max(list.length - 1, 0);
	const start = Number.isInteger(requested) ? Math.min(Math.max(requested, 0), last) : 0;
	return { archive: book, pages: list, start };
};
