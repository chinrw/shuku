import { describe, expect, it } from 'vitest';
import { route } from './routes';

const origin = 'https://shuku.example.ts.net';
const get = (path: string, mode = 'cors') => ({ method: 'GET', mode, url: origin + path });

describe('route', () => {
	it('serves navigations network first, whatever the path', () => {
		expect(route(get('/a/7/read?page=3', 'navigate'), origin)).toBe('navigate');
		expect(route(get('/login', 'navigate'), origin)).toBe('navigate');
	});

	it('caches page images and thumbnails only when the URL carries the content hash', () => {
		expect(route(get('/api/archives/7/pages/3?v=abc', 'no-cors'), origin)).toBe('page');
		expect(route(get('/api/archives/7/pages/3?v=abc&w=1080', 'no-cors'), origin)).toBe('page');
		expect(route(get('/api/archives/7/pages/3/thumbnail?v=abc'), origin)).toBe('thumbnail');
		expect(route(get('/api/archives/7/thumbnail?v=abc'), origin)).toBe('thumbnail');
		expect(route(get('/api/archives/7/pages/3'), origin)).toBeNull();
		expect(route(get('/api/archives/7/thumbnail'), origin)).toBeNull();
	});

	it('keeps the reading and library JSON for offline use', () => {
		for (const path of [
			'/api/search?q=&sort=date',
			'/api/recent',
			'/api/categories',
			'/api/saved-tags',
			'/api/series/2',
			'/api/archives/7',
			'/api/archives/7/pages',
			'/api/archives/7/similar'
		])
			expect(route(get(path), origin), path).toBe('api');
	});

	it('leaves admin, auth, server info, downloads, MCP and writes alone', () => {
		for (const path of [
			'/api/admin/status',
			'/api/auth',
			'/api/info',
			'/api/tags?q=a',
			'/api/archives/7/file',
			'/api/archives/7/metadata',
			'/mcp'
		])
			expect(route(get(path), origin), path).toBeNull();
		expect(route({ method: 'PUT', mode: 'cors', url: `${origin}/api/archives/7/progress` }, origin)).toBeNull();
		expect(route({ method: 'POST', mode: 'navigate', url: `${origin}/login` }, origin)).toBeNull();
	});

	it('watches logout so the cached library can be dropped', () => {
		expect(route({ method: 'DELETE', mode: 'cors', url: `${origin}/api/auth/session` }, origin)).toBe(
			'logout'
		);
		expect(route({ method: 'POST', mode: 'cors', url: `${origin}/api/auth/session` }, origin)).toBeNull();
	});

	it('ignores other origins and treats the rest as the app shell', () => {
		expect(route(get('/api/recent'), 'https://other.example')).toBeNull();
		expect(route(get('/_app/immutable/entry/app.js'), origin)).toBe('static');
		expect(route(get('/manifest.webmanifest'), origin)).toBe('static');
	});
});
