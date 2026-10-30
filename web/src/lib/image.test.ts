import { describe, expect, it } from 'vitest';
import { fittedWidth, pageUrl, widthTier } from './image';

describe('widthTier', () => {
	it('picks the smallest tier that covers the device pixels', () => {
		expect(widthTier(390, 3)).toBe(1440);
		expect(widthTier(360, 2)).toBe(720);
		expect(widthTier(361, 2)).toBe(1080);
		expect(widthTier(1920, 1)).toBe(2160);
	});

	it('caps at the largest tier', () => {
		expect(widthTier(2560, 2)).toBe(2160);
	});
});

describe('pageUrl', () => {
	const page = { width: 1200, height: 1700, url: '/api/archives/1/pages/0?v=abc' };

	it('adds the width to the server-provided url', () => {
		expect(pageUrl(page, 1080)).toBe('/api/archives/1/pages/0?v=abc&w=1080');
	});

	it('uses the original when the tier is not narrower than the page', () => {
		expect(pageUrl(page, 1440)).toBe(page.url);
		expect(pageUrl({ ...page, width: 1080 }, 1080)).toBe(page.url);
	});
});

describe('fittedWidth', () => {
	it('is limited by the viewport height for tall pages', () => {
		expect(fittedWidth({ width: 1000, height: 2000 }, { width: 800, height: 1000 })).toBe(500);
	});

	it('is limited by the viewport width for wide pages', () => {
		expect(fittedWidth({ width: 2000, height: 1000 }, { width: 800, height: 1000 })).toBe(800);
	});
});
