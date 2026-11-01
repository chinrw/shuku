import { describe, expect, it } from 'vitest';
import { WheelTurner } from './wheelTurn';

describe('WheelTurner', () => {
	it('turns one page per mouse notch', () => {
		const turner = new WheelTurner();
		expect(turner.feed(100, 0, true)).toBe(1);
		expect(turner.feed(100, 500, true)).toBe(1);
		expect(turner.feed(-100, 1000, true)).toBe(-1);
	});

	it('turns on a single notch reported in lines', () => {
		const turner = new WheelTurner();
		expect(turner.feed(3, 0, true, true)).toBe(1);
		expect(turner.feed(-3, 500, true, true)).toBe(-1);
	});

	it('turns once for a touchpad swipe and its momentum', () => {
		const turner = new WheelTurner();
		const turns: number[] = [];
		// 每 16 毫秒一个事件，先加速再随惯性慢慢变小，持续一秒多。
		for (let i = 0; i < 70; i++) {
			const delta = i < 10 ? 10 + i * 4 : Math.max(1, 50 * 0.93 ** (i - 10));
			turns.push(turner.feed(delta, i * 16, true));
		}
		expect(turns.filter((turn) => turn !== 0)).toEqual([1]);
	});

	it('turns once for a long, fast swipe', () => {
		const turner = new WheelTurner();
		const turns: number[] = [];
		for (let i = 0; i < 60; i++) turns.push(turner.feed(60, i * 16, true));
		expect(turns.filter((turn) => turn !== 0)).toEqual([1]);
	});

	it('turns once while the wheel spins without a pause', () => {
		const turner = new WheelTurner();
		const turns: number[] = [];
		for (let i = 0; i < 10; i++) turns.push(turner.feed(100, i * 100, true));
		expect(turns.filter((turn) => turn !== 0)).toEqual([1]);
	});

	it('ignores small jitter', () => {
		const turner = new WheelTurner();
		expect(turner.feed(4, 0, true)).toBe(0);
		expect(turner.feed(4, 1000, true)).toBe(0);
	});

	it('does not turn when a scroll reaches the end of the page', () => {
		const turner = new WheelTurner();
		expect(turner.feed(100, 0, false)).toBe(0);
		expect(turner.feed(100, 100, true)).toBe(0);
		// 停下来再滚一次才翻。
		expect(turner.feed(100, 400, true)).toBe(1);
	});
});
