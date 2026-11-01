import type { TransitionConfig } from 'svelte/transition';

// 网页端只有两种动画：面板从屏幕边缘滑进来（遮罩同时淡入），和阅读器翻页时的左右滑动。
// 系统设置了减少动态效果时都不播放。

/** 和 CSS 的 cubic-bezier() 相同的缓动曲线。 */
export function cubicBezier(x1: number, y1: number, x2: number, y2: number): (x: number) => number {
	const at = (a: number, b: number, t: number) =>
		3 * a * t * (1 - t) ** 2 + 3 * b * t ** 2 * (1 - t) + t ** 3;
	return (x) => {
		if (x <= 0 || x >= 1) return x;
		// x 随 t 单调增加，二分 20 次的误差在 1e-6 以内。
		let low = 0;
		let high = 1;
		for (let i = 0; i < 20; i++) {
			const t = (low + high) / 2;
			if (at(x1, x2, t) < x) low = t;
			else high = t;
		}
		return at(y1, y2, (low + high) / 2);
	};
}

const easeOut = cubicBezier(0.2, 0.8, 0.2, 1);

function duration(ms: number): number {
	return window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : ms;
}

// 退场中的元素由 Svelte 设成 inert，不挡点击；中途又进场时 Svelte 会复用它并取消 inert。
type Options = { direction?: 'in' | 'out' | 'both' };

/** 宽屏的面板是右侧抽屉，手机上是底部面板，和各处 CSS 里 600px 的断点一致。 */
export function panelEdge(): 'bottom' | 'right' {
	return window.matchMedia('(max-width: 599px)').matches ? 'bottom' : 'right';
}

/** 面板从 `from` 那一边滑进来，关上时滑回去。`from` 为 null 时不动，例如宽屏上的标签浮层。 */
export function slide(_node: Element, { from }: { from: 'bottom' | 'right' | null }): TransitionConfig {
	const axis = from === 'bottom' ? 'Y' : 'X';
	return {
		duration: from ? duration(320) : 0,
		easing: easeOut,
		css: (t) => `transform: translate${axis}(${(1 - t) * 100}%)`
	};
}

export function fade(_node: Element): TransitionConfig {
	return { duration: duration(280), css: (t) => `opacity: ${t}` };
}

/**
 * 翻页：新的一屏从一侧滑进来，旧的一屏往另一侧滑出去，两屏叠在同一个框里。`sign` 为 1 时
 * 内容往左移，新的一屏从右边进来；-1 相反；0 直接换。
 */
export function turn(
	node: Element,
	{ sign }: { sign: number },
	options?: Options
): TransitionConfig | (() => TransitionConfig) {
	const ms = sign ? duration(280) : 0;
	if (options?.direction !== 'out') {
		return {
			duration: ms,
			easing: easeOut,
			css: (t) => `transform: translateX(${(1 - t) * sign * 100}%)`
		};
	}
	// 连续快翻时退场的一屏可能还没滑到位。Svelte 的退场总从 t=1 开始，所以在退场开始时
	// （Svelte 调用这个函数时）读它眼下的位置，从那里接着滑出去，不先跳回中间。
	return () => {
		const transform = getComputedStyle(node).transform;
		const from = transform === 'none' ? 0 : new DOMMatrixReadOnly(transform).m41;
		return {
			duration: ms,
			easing: easeOut,
			css: (t) => `transform: translateX(calc(${from * t}px + ${(1 - t) * -sign * 100}%))`
		};
	};
}
