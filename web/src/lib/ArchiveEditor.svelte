<script lang="ts">
	// 详情页里改标题、标签、来源和简介。只提交改过的字段，没改的不覆盖，
	// 插件同时补上的标签不会被这里的旧值盖掉，除非这次也改了标签。
	import { ApiError, editArchive, type ArchiveDetail, type ArchiveEdit } from '#lib/api.ts';
	import { parseTagText, tagText } from '#lib/tags.ts';
	import { untrack } from 'svelte';

	let {
		book,
		onsaved,
		oncancel
	}: { book: ArchiveDetail; onsaved: () => Promise<void>; oncancel: () => void } = $props();

	// 打开编辑时的值，之后不跟着 `book` 变；详情页换书时会收起编辑框。
	const original = untrack(() => ({
		title: book.title,
		tags: tagText(book.tags),
		source: book.source_url ?? '',
		summary: book.summary
	}));
	let title = $state(original.title);
	let tags = $state(original.tags);
	let source = $state(original.source);
	let summary = $state(original.summary);
	let saving = $state(false);
	let error = $state('');

	function changes(): ArchiveEdit {
		const edit: ArchiveEdit = {};
		if (title.trim() !== original.title) edit.title = title.trim();
		if (summary !== original.summary) edit.summary = summary;
		if (source.trim() !== original.source) edit.source_url = source.trim();
		// 只比较拆分后的结果，光改了空格或逗号后的空白不算改动。
		const parsed = parseTagText(tags);
		if (parsed.join('\n') !== parseTagText(original.tags).join('\n')) edit.tags = parsed;
		return edit;
	}

	async function save(event: SubmitEvent) {
		event.preventDefault();
		if (saving) return;
		const edit = changes();
		if (Object.keys(edit).length === 0) {
			oncancel();
			return;
		}
		const target = book.id;
		saving = true;
		error = '';
		try {
			await editArchive(target, edit);
			// 等详情重新读完再放开按钮，免得这期间又点开一次编辑。
			await onsaved();
		} catch (cause) {
			error = cause instanceof ApiError ? cause.message : '网络错误';
		} finally {
			saving = false;
		}
	}
</script>

<form class="editor" onsubmit={save}>
	<!-- 保存期间锁住输入：请求发出后再改的内容不会被提交，保存完表单就收起了。 -->
	<fieldset disabled={saving}>
		<label>
			<span>标题</span>
			<input bind:value={title} required maxlength="1000" />
		</label>
		<label>
			<span>标签</span>
			<textarea bind:value={tags} rows="5" spellcheck="false"></textarea>
			<small>用英文逗号分隔，带命名空间的写成 artist:某某。</small>
		</label>
		<label>
			<span>来源</span>
			<input bind:value={source} inputmode="url" spellcheck="false" placeholder="没有" />
		</label>
		<label>
			<span>简介</span>
			<textarea bind:value={summary} rows="4"></textarea>
		</label>
		<p class="path">文件：{book.path}</p>
		{#if error}<p class="error" role="alert">没有保存：{error}</p>{/if}
	</fieldset>
	<div class="buttons">
		<button class="primary" disabled={saving || !title.trim()}>保存</button>
		<button type="button" onclick={oncancel} disabled={saving}>取消</button>
	</div>
</form>

<style>
	.editor {
		display: grid;
		gap: 0.75rem;
		padding: 0.75rem;
		border-radius: 9px;
		background: var(--surface);
	}

	fieldset {
		display: grid;
		gap: 0.75rem;
		min-width: 0;
		margin: 0;
		padding: 0;
		border: none;
	}

	label {
		display: grid;
		gap: 0.3rem;
	}

	label span {
		color: var(--muted);
		font-size: 0.8rem;
	}

	input,
	textarea {
		width: 100%;
		box-sizing: border-box;
		padding: 0.45rem 0.6rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: var(--sunken);
		color: inherit;
		font: inherit;
	}

	textarea {
		resize: vertical;
		line-height: 1.5;
	}

	small,
	.path {
		margin: 0;
		color: var(--faint);
		font-size: 0.78rem;
		overflow-wrap: anywhere;
	}

	.error {
		margin: 0;
	}

	.buttons {
		display: flex;
		gap: 0.5rem;
	}

	button {
		padding: 0.5rem 0.9rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: var(--surface);
		color: inherit;
		font: inherit;
		cursor: pointer;
	}

	button.primary {
		border-color: var(--accent);
		background: var(--accent);
		color: white;
	}

	button:disabled {
		opacity: 0.6;
		cursor: default;
	}
</style>
