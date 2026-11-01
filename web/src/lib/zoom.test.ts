import { describe, expect, it } from 'vitest';
import { MAX_ZOOM, NO_ZOOM, panBy, zoomAt } from './zoom';

describe('zoomAt', () => {
	it('keeps the point under the cursor in place', () => {
		const zoom = zoomAt(NO_ZOOM, 2, 300, 200, 1000, 800);
		expect(zoom).toEqual({ scale: 2, x: -300, y: -200 });
		// 屏幕上的 (300, 200) 仍然对应内容里的 (300, 200)。
		expect((300 - zoom.x) / zoom.scale).toBe(300);
		expect((200 - zoom.y) / zoom.scale).toBe(200);
	});

	it('limits the scale to between 1 and MAX_ZOOM', () => {
		expect(zoomAt(NO_ZOOM, 0.5, 300, 200, 1000, 800)).toEqual(NO_ZOOM);
		expect(zoomAt(NO_ZOOM, 100, 300, 200, 1000, 800).scale).toBe(MAX_ZOOM);
	});

	it('returns to no zoom when zooming all the way out', () => {
		const zoomed = zoomAt(NO_ZOOM, 3, 900, 700, 1000, 800);
		expect(zoomAt(zoomed, 1 / 3, 10, 10, 1000, 800)).toEqual(NO_ZOOM);
	});
});

describe('panBy', () => {
	it('moves the view', () => {
		const zoom = zoomAt(NO_ZOOM, 2, 500, 400, 1000, 800);
		expect(panBy(zoom, 100, -50, 1000, 800)).toEqual({ scale: 2, x: -400, y: -450 });
	});

	it('does not pan past the edges of the content', () => {
		const zoom = zoomAt(NO_ZOOM, 2, 500, 400, 1000, 800);
		expect(panBy(zoom, 5000, 5000, 1000, 800)).toEqual({ scale: 2, x: 0, y: 0 });
		expect(panBy(zoom, -5000, -5000, 1000, 800)).toEqual({ scale: 2, x: -1000, y: -800 });
	});

	it('does nothing without zoom', () => {
		expect(panBy(NO_ZOOM, 100, 100, 1000, 800)).toEqual(NO_ZOOM);
	});
});
