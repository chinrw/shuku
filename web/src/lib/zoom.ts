// 阅读器在电脑上的放大：整屏按 transform 的 translate(x, y) scale(scale) 变换，
// 变换原点在图片区域的左上角。坐标都是相对图片区域的 CSS 像素。

export interface Zoom {
	scale: number;
	x: number;
	y: number;
}

export const NO_ZOOM: Zoom = { scale: 1, x: 0, y: 0 };
export const MAX_ZOOM = 8;

/** 限制平移，放大后的画面始终盖满宽 width、高 height 的区域，不会拖出空白。 */
function clamp(zoom: Zoom, width: number, height: number): Zoom {
	if (zoom.scale <= 1) return NO_ZOOM;
	const x = Math.min(0, Math.max(width - width * zoom.scale, zoom.x));
	const y = Math.min(0, Math.max(height - height * zoom.scale, zoom.y));
	return { scale: zoom.scale, x, y };
}

/** 以 (px, py) 为中心把倍数乘上 factor，倍数限制在 1 到 MAX_ZOOM，鼠标下的那一点不动。 */
export function zoomAt(
	zoom: Zoom,
	factor: number,
	px: number,
	py: number,
	width: number,
	height: number
): Zoom {
	const scale = Math.min(MAX_ZOOM, Math.max(1, zoom.scale * factor));
	const ratio = scale / zoom.scale;
	return clamp({ scale, x: px - (px - zoom.x) * ratio, y: py - (py - zoom.y) * ratio }, width, height);
}

/** 把画面平移 (dx, dy)。 */
export function panBy(zoom: Zoom, dx: number, dy: number, width: number, height: number): Zoom {
	return clamp({ scale: zoom.scale, x: zoom.x + dx, y: zoom.y + dy }, width, height);
}
