import { archive, categories, pages } from '#lib/api.ts';
import { orNotFound, parseId } from '#lib/load.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params, url }) => {
	const id = parseId(params.id);
	const [book, list, categoryList] = await orNotFound(
		Promise.all([archive(fetch, id), pages(fetch, id), categories(fetch)])
	);
	const requested = Number(url.searchParams.get('page') ?? book.progress?.page ?? 0);
	const last = Math.max(list.pages.length - 1, 0);
	const start = Number.isInteger(requested) ? Math.min(Math.max(requested, 0), last) : 0;
	return {
		archive: book,
		pages: list.pages,
		webtoon: list.webtoon,
		start,
		categories: categoryList
	};
};
