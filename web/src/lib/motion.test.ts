import { expect, test } from 'vitest';
import { cubicBezier } from './motion';

test('cubicBezier matches the endpoints and stays monotonic', () => {
	const ease = cubicBezier(0.2, 0.8, 0.2, 1);
	expect(ease(0)).toBe(0);
	expect(ease(1)).toBe(1);
	let last = 0;
	for (let x = 0.05; x < 1; x += 0.05) {
		const y = ease(x);
		expect(y).toBeGreaterThan(last);
		last = y;
	}
	// 前快后慢：走到一半时间已经过了大部分路程。
	expect(ease(0.5)).toBeGreaterThan(0.85);
});

test('cubicBezier with linear control points is the identity', () => {
	const linear = cubicBezier(1 / 3, 1 / 3, 2 / 3, 2 / 3);
	for (const x of [0.1, 0.25, 0.5, 0.9]) expect(linear(x)).toBeCloseTo(x, 5);
});
