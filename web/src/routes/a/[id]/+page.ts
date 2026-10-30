import { archive } from '#lib/api.ts';
import { orNotFound, parseId } from '#lib/load.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params }) => ({
	archive: await orNotFound(archive(fetch, parseId(params.id)))
});
