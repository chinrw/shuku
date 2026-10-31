import { archive, categories } from '#lib/api.ts';
import { orNotFound, parseId } from '#lib/load.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params }) => {
	const [book, categoryList] = await orNotFound(
		Promise.all([archive(fetch, parseId(params.id)), categories(fetch)])
	);
	return { archive: book, categories: categoryList };
};
