/**
 * 点击标签时生成精确匹配的搜索词条。引号能包住逗号，所以优先用引号；
 * 搜索语法不支持转义，值里本身有双引号时只能改用末尾的 `$`。
 */
export function tagTerm(namespace: string, value: string): string {
	const tag = namespace ? `${namespace}:${value}` : value;
	return tag.includes('"') ? `${tag}$` : `"${tag}"`;
}
