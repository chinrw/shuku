import { archive, categories, pages } from '#lib/api.ts';
import { orNotFound, parseId } from '#lib/load.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params }) => {
	const id = parseId(params.id);
	const [book, list, categoryList] = await orNotFound(
		Promise.all([archive(fetch, id), pages(fetch, id), categories(fetch)])
	);
	return { archive: book, pages: list, categories: categoryList };
};
