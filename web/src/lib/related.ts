import type { TagGroup } from './api';
import { tagTerm } from './query';

export interface RelatedQuery {
	label: string;
	term: string;
}

// 推荐依据的命名空间和显示名，按顺序取，最多 MAX_ROWS 排，免得一本书打开时发出太多请求。
// 按卷出版的漫画靠 series 和 publisher，同人志靠 group、circle 和 parody；
// series 排第一，这样多卷的作品先列出同系列的其他卷。
const SOURCES: [string, string][] = [
	['series', '系列'],
	['artist', '作者'],
	['group', '社团'],
	['circle', '社团'],
	['publisher', '出版社'],
	['parody', '原作']
];
const MAX_ROWS = 4;
// 原创作品都打 parody:original，按它推荐等于随机推荐。
const SKIPPED = new Set(['parody:original', 'series:original']);

export function relatedQueries(groups: TagGroup[]): RelatedQuery[] {
	const queries: RelatedQuery[] = [];
	for (const [namespace, label] of SOURCES) {
		const group = groups.find((candidate) => candidate.namespace.toLowerCase() === namespace);
		for (const value of group?.values ?? []) {
			if (SKIPPED.has(`${namespace}:${value.toLowerCase()}`)) continue;
			queries.push({ label: `${label}：${value}`, term: tagTerm(group!.namespace, value) });
		}
	}
	return queries.slice(0, MAX_ROWS);
}
