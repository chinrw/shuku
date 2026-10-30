import type { TagGroup } from './api';

// 卡片和列表先显示这些命名空间，其余按名字排序，没有命名空间的标签放最后。
// 顺序参照 LRR 着色的几类。
const PRIORITY = ['artist', 'group', 'circle', 'parody', 'series', 'character'];

export function orderGroups(groups: TagGroup[]): TagGroup[] {
	const rank = (namespace: string) => {
		if (namespace === '') return PRIORITY.length + 1;
		const index = PRIORITY.indexOf(namespace.toLowerCase());
		return index === -1 ? PRIORITY.length : index;
	};
	return [...groups].sort(
		(a, b) => rank(a.namespace) - rank(b.namespace) || a.namespace.localeCompare(b.namespace)
	);
}

/** 沿用 LRR 的着色：作者蓝、社团青绿、原作粉，其余标签用默认颜色。 */
export function namespaceClass(namespace: string): string {
	switch (namespace.toLowerCase()) {
		case 'artist':
			return 'ns-artist';
		case 'group':
		case 'circle':
			return 'ns-group';
		case 'parody':
		case 'series':
			return 'ns-parody';
		default:
			return '';
	}
}
