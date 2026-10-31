import { describe, expect, it } from 'vitest';
import { handoffToken, serverListHref } from './nativeApp';

describe('serverListHref', () => {
	it('is null outside the app', () => {
		expect(serverListHref('http://nas:3002/?q=x', undefined)).toBeNull();
	});

	it('passes the current page to the server list', () => {
		const href = serverListHref('http://nas:3002/?q=a&p=2', 'tauri://localhost/index.html');
		expect(href).toMatch(/^tauri:\/\/localhost\/index\.html\?from=/);
		const url = new URL(href!);
		expect(url.searchParams.get('from')).toBe('http://nas:3002/?q=a&p=2');
	});
});

describe('handoffToken', () => {
	it('reads the token from the fragment', () => {
		expect(handoffToken('#token=shuku_abc')).toBe('shuku_abc');
		expect(handoffToken('token=shuku_abc')).toBe('shuku_abc');
	});

	it('is null without a token', () => {
		expect(handoffToken('')).toBeNull();
		expect(handoffToken('#next=/')).toBeNull();
	});
});
