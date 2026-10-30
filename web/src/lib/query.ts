/**
 * 点击标签时生成精确匹配的搜索词条。整个标签放在引号里，引号能包住逗号；
 * 反斜杠、引号和通配符 `* % ? _` 前面补一个反斜杠，服务端把转义后的字符
 * 当普通字符，所以值里含 `_` 的标签也能精确匹配。
 */
export function tagTerm(namespace: string, value: string): string {
	const tag = namespace ? `${namespace}:${value}` : value;
	return `"${tag.replace(/[\\"*%?_]/g, '\\$&')}"`;
}

/** 词条末尾的 `ch` 前面有奇数个反斜杠时它是被转义的普通字符，不算语法。 */
function endsUnescaped(term: string, ch: string): boolean {
	if (!term.endsWith(ch)) return false;
	let backslashes = 0;
	for (let i = term.length - 2; i >= 0 && term[i] === '\\'; i--) backslashes++;
	return backslashes % 2 === 0;
}

/**
 * 拆出搜索语句的最后一个词条，供补全使用。`head` 是它前面的部分（含最后一个
 * 不在引号里的逗号），`text` 去掉了开头的 `-` 和引号，并还原了反斜杠转义。
 * 词条已经用未转义的引号或 `$` 收尾时 `done` 为真，不再补全。
 * 逗号和引号的识别与服务端一致：转义的引号不切换引号状态，转义的逗号不分词。
 */
export function lastTerm(input: string): {
	head: string;
	text: string;
	excluded: boolean;
	done: boolean;
} {
	let quoted = false;
	let escaped = false;
	let cut = 0;
	for (let i = 0; i < input.length; i++) {
		if (escaped) escaped = false;
		else if (input[i] === '\\') escaped = true;
		else if (input[i] === '"') quoted = !quoted;
		else if (input[i] === ',' && !quoted) cut = i + 1;
	}
	const rest = input.slice(cut).trim();
	const excluded = rest.startsWith('-');
	const term = (excluded ? rest.slice(1) : rest).trim();
	const done =
		endsUnescaped(term, '$') ||
		(term.length > 1 && term.startsWith('"') && endsUnescaped(term, '"'));
	const text = term.replace(/^"/, '').replace(/\\(.)/g, '$1');
	return { head: input.slice(0, cut), text, excluded, done };
}

/** 用补全选中的标签替换最后一个词条，保留排除标记，末尾留好逗号方便接着输入。 */
export function completeTerm(input: string, tag: string): string {
	const { head, excluded } = lastTerm(input);
	const before = head ? `${head} ` : '';
	return `${before}${excluded ? '-' : ''}${tagTerm('', tag)}, `;
}

/**
 * 去掉语句末尾的分隔逗号。只有不在引号里、没有被转义的逗号才是分隔符，
 * `\,` 是词条的一部分，不能当成分隔符删掉。
 */
export function stripTrailingSeparator(input: string): string {
	const trimmed = input.trim();
	const { head } = lastTerm(trimmed);
	return head.length === trimmed.length ? head.slice(0, -1).trimEnd() : trimmed;
}

/** 在搜索语句末尾加上一个精确匹配的标签条件。 */
export function addTerm(input: string, tag: string): string {
	const rest = stripTrailingSeparator(input);
	if (!rest) return tagTerm('', tag);
	// 末尾孤立的反斜杠是普通字符，补一个写成 `\\`，免得它把新加的分隔逗号转义掉。
	const literal = endsUnescaped(rest, '\\') ? `${rest}\\` : rest;
	return `${literal}, ${tagTerm('', tag)}`;
}
