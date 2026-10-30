import { describe, expect, it } from 'vitest';
import { httpUrl } from './url';

describe('httpUrl', () => {
	it('keeps http and https links', () => {
		expect(httpUrl('https://example.com/g/1')).toBe('https://example.com/g/1');
		expect(httpUrl('http://example.com')).toBe('http://example.com/');
	});

	it('rejects other schemes and text that is not a url', () => {
		expect(httpUrl('javascript:alert(1)')).toBeNull();
		expect(httpUrl(' JavaScript:alert(1)')).toBeNull();
		expect(httpUrl('data:text/html,hi')).toBeNull();
		expect(httpUrl('example.com/g/1')).toBeNull();
	});
});
