/**
 * 点击标签时生成精确匹配的搜索词条。引号能包住逗号，所以优先用引号；
 * 搜索语法不支持转义，值里本身有双引号时只能改用末尾的 `$`。
 */
export function tagTerm(namespace: string, value: string): string {
	const tag = namespace ? `${namespace}:${value}` : value;
	return tag.includes('"') ? `${tag}$` : `"${tag}"`;
}

/**
 * 拆出搜索语句的最后一个词条，供补全使用。`head` 是它前面的部分（含最后一个
 * 不在引号里的逗号），`text` 去掉了开头的 `-` 和引号。词条已经用引号或 `$`
 * 收尾时 `done` 为真，不再补全。
 */
export function lastTerm(input: string): {
	head: string;
	text: string;
	excluded: boolean;
	done: boolean;
} {
	let quoted = false;
	let cut = 0;
	for (let i = 0; i < input.length; i++) {
		if (input[i] === '"') quoted = !quoted;
		else if (input[i] === ',' && !quoted) cut = i + 1;
	}
	const rest = input.slice(cut).trim();
	const excluded = rest.startsWith('-');
	const term = (excluded ? rest.slice(1) : rest).trim();
	const done = term.endsWith('$') || (term.length > 1 && term.startsWith('"') && term.endsWith('"'));
	return { head: input.slice(0, cut), text: term.replace(/^"/, ''), excluded, done };
}

/** 用补全选中的标签替换最后一个词条，保留排除标记，末尾留好逗号方便接着输入。 */
export function completeTerm(input: string, tag: string): string {
	const { head, excluded } = lastTerm(input);
	const before = head ? `${head} ` : '';
	return `${before}${excluded ? '-' : ''}${tagTerm('', tag)}, `;
}

/** 在搜索语句末尾加上一个精确匹配的标签条件。 */
export function addTerm(input: string, tag: string): string {
	const rest = input.trim().replace(/,$/, '').trimEnd();
	return rest ? `${rest}, ${tagTerm('', tag)}` : tagTerm('', tag);
}
