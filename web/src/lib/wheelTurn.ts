// 滚轮翻页：把一串滚轮事件换成一次次翻页。一次滚动（事件之间不停顿超过 GAP）最多翻一页：
// 鼠标滚一格翻一页，触控板一次滑动连同松手后的惯性也只翻一页，快速连续滚滚轮不会一下翻过好几页。

/** 两次滚轮事件隔这么久以上，算新的一次滚动。 */
const GAP = 150;
/** 按像素上报时累计滚动这么多翻一页。按行、按页上报（Firefox 的鼠标滚轮）的一格就够。 */
const THRESHOLD = 50;

export class WheelTurner {
	private last = -Infinity;
	private turned = false;
	private sum = 0;
	private startedAtEdge = true;

	/**
	 * 喂一个滚轮事件，返回 1（下一页）、-1（上一页）或 0。`discrete` 是按行或按页上报的
	 * 事件。`edge` 是这一屏在滚动方向上已经到头；适应宽度时只有从头开始的一次滚动才翻页，
	 * 滚到底的惯性不会顺带翻过去。
	 */
	feed(delta: number, time: number, edge: boolean, discrete = false): -1 | 0 | 1 {
		if (time - this.last >= GAP) {
			this.sum = 0;
			this.turned = false;
			this.startedAtEdge = edge;
		}
		this.last = time;
		if (this.turned || !this.startedAtEdge || !edge) return 0;
		this.sum += discrete ? Math.sign(delta) * THRESHOLD : delta;
		if (Math.abs(this.sum) < THRESHOLD) return 0;
		this.turned = true;
		return this.sum > 0 ? 1 : -1;
	}
}
