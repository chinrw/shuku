import type { TagGroup } from './api';
import { tagTerm } from './query';
import { httpUrl } from './url';

// 标签的分组、排序和显示照搬 LRR（public/js/mod/common.js 的 buildTagsDiv 和 colorCodeTags）。

export interface TagValue {
	text: string;
	/** 普通标签链接到精确搜索，来源是外链，加入日期不可点。 */
	href: string | null;
	external: boolean;
}

export interface TagRow {
	namespace: string;
	label: string;
	values: TagValue[];
}

/** LRR 把以 date 或 time 开头的命名空间当作 Unix 秒显示成日期。 */
const TIME = /^(date|time)/;

function formatDate(seconds: number): string {
	return new Date(seconds * 1000).toLocaleDateString();
}

function sourceValue(raw: string): TagValue {
	const href = httpUrl(/^https?:\/\//.test(raw) ? raw : `https://${raw}`);
	return { text: raw, href, external: true };
}

function tagValue(namespace: string, value: string): TagValue {
	if (namespace === 'source') return sourceValue(value);
	const seconds = Number(value);
	const text = TIME.test(namespace) && Number.isFinite(seconds) ? formatDate(seconds) : value;
	return { text, href: `/?q=${encodeURIComponent(tagTerm(namespace, value))}`, external: false };
}

// 没有命名空间的标签在 LRR 里归入 other。
function groupKey(namespace: string): string {
	return namespace || 'other';
}

function label(key: string): string {
	return key === 'date_added' ? 'Date Added' : key.charAt(0).toUpperCase() + key.slice(1);
}

/**
 * LRR 标签表的行。Shuku 把加入时间和来源存成归档的列，这里还原成 LRR 里的
 * date_added 和 source 两行。命名空间按名字排序。
 */
export function tagRows(item: {
	tags: TagGroup[];
	date_added: number;
	source_url: string | null;
}): TagRow[] {
	const rows = new Map<string, TagValue[]>();
	const add = (key: string, value: TagValue) => rows.set(key, [...(rows.get(key) ?? []), value]);
	for (const group of item.tags) {
		for (const value of group.values) add(groupKey(group.namespace), tagValue(group.namespace, value));
	}
	add('date_added', { text: formatDate(item.date_added), href: null, external: false });
	if (item.source_url) add('source', sourceValue(item.source_url));
	return [...rows.keys()]
		.sort()
		.map((key) => ({ namespace: key, label: label(key), values: rows.get(key)! }));
}

/** 卡片底部那一行：按命名空间名排序，只显示值，不显示时间类命名空间。 */
export function tagLine(groups: TagGroup[]): { text: string; namespace: string }[] {
	return [...groups]
		.filter((group) => !TIME.test(group.namespace))
		.sort((a, b) => (groupKey(a.namespace) < groupKey(b.namespace) ? -1 : 1))
		.flatMap((group) => group.values.map((text) => ({ text, namespace: group.namespace })));
}

/** LRR 只给这几类命名空间着色：作者蓝、社团青绿、原作粉。 */
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

/** 编辑框里的标签：和 LRR 一样写成 `命名空间:值`，用逗号分隔，按存储层给出的顺序。 */
export function tagText(groups: TagGroup[]): string {
	return groups
		.flatMap((group) =>
			group.values.map((value) => (group.namespace ? `${group.namespace}:${value}` : value))
		)
		.join(', ');
}

/**
 * 把编辑框里的文字拆成标签，只按英文逗号分隔，和 LRR 一样，空项去掉。全角逗号和换行
 * 都不算分隔：已有的标签值里可能有全角逗号；换行由服务端拒绝，不会被悄悄拆成两个标签。
 */
export function parseTagText(text: string): string[] {
	return text
		.split(',')
		.map((tag) => tag.trim())
		.filter((tag) => tag !== '');
}
