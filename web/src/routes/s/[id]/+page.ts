import { series } from '#lib/api.ts';
import { orNotFound, parseId } from '#lib/load.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params }) => {
	return { series: await orNotFound(series(fetch, parseId(params.id))) };
};
