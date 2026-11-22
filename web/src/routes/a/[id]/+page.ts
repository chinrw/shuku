import { redirect } from '@sveltejs/kit';
import { archive, categories, pages } from '#lib/api.ts';
import { orNotFound, parseId } from '#lib/load.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params, url }) => {
	const id = parseId(params.id);
	const [book, list, categoryList] = await orNotFound(
		Promise.all([archive(fetch, id), pages(fetch, id), categories(fetch)])
	);
	if (book.id !== id) redirect(307, `/a/${book.id}${url.search}`);
	return { archive: book, pages: list.pages, categories: categoryList };
};
