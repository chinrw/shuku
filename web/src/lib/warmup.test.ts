import { expect, test } from 'vitest';
import { uncachedApiRequests } from './warmup';

const ORIGIN = 'http://shuku.local';

test('keeps read APIs fetched before the takeover that the service worker caches, once each', () => {
	const entries = [
		{ name: `${ORIGIN}/api/search?q=a`, initiatorType: 'fetch', startTime: 10 },
		{ name: `${ORIGIN}/api/search?q=a`, initiatorType: 'fetch', startTime: 10 },
		{ name: `${ORIGIN}/api/archives/3/pages`, initiatorType: 'fetch', startTime: 10 },
		{ name: `${ORIGIN}/api/archives/3/thumbnail?v=ab`, initiatorType: 'img', startTime: 10 },
		{ name: `${ORIGIN}/api/archives/3/progress`, initiatorType: 'fetch', startTime: 10 },
		{ name: `${ORIGIN}/api/admin/status`, initiatorType: 'fetch', startTime: 10 },
		{ name: `http://elsewhere/api/search`, initiatorType: 'fetch', startTime: 10 },
		{ name: `${ORIGIN}/api/recent`, initiatorType: 'fetch', startTime: 200 },
		{ name: `${ORIGIN}/_app/immutable/start.js`, initiatorType: 'script', startTime: 10 }
	];
	expect(uncachedApiRequests(entries, ORIGIN, 100)).toEqual([
		`${ORIGIN}/api/search?q=a`,
		`${ORIGIN}/api/archives/3/pages`
	]);
});
