<script lang="ts">
	import { namespaces as loadNamespaces, renameTag, type NamespaceCount } from '#lib/admin.ts';
	import { suggestTags, type TagCount } from '#lib/api.ts';
	import { count, percent } from '#lib/format.ts';
	import { tagTerm } from '#lib/query.ts';
	import { namespaceClass } from '#lib/tags.ts';

	const TOP = 30;

	let list = $state<NamespaceCount[] | null>(null);
	let selected = $state<string | null>(null);
	let top = $state<TagCount[]>([]);
	let error = $state<string | null>(null);
	// 改名后递增，让最常用标签的列表重新取。
	let version = $state(0);

	let from = $state('');
	let to = $state('');
	let renaming = $state(false);
	let renameNote = $state('');

	function message(cause: unknown) {
		return cause instanceof Error ? cause.message : String(cause);
	}

	async function load() {
		try {
			const loaded = await loadNamespaces();
			list = loaded;
			if (!loaded.some((item) => item.namespace === selected)) {
				selected = loaded.find((item) => item.namespace !== '')?.namespace ?? null;
			}
		} catch (cause) {
			error = message(cause);
		}
	}

	$effect(() => {
		load();
	});

	async function rename(event: SubmitEvent) {
		event.preventDefault();
		const source = from.trim();
		const target = to.trim();
		if (!source || !target || renaming) return;
		if (!confirm(`把标签“${source}”改成“${target}”？目标已经存在时两者合并，不能撤销。`)) return;
		renaming = true;
		renameNote = '';
		try {
			const { archives } = await renameTag(source, target);
			renameNote = `已改名，涉及 ${count(archives)} 本。`;
			from = '';
			to = '';
			await load();
			version += 1;
		} catch (cause) {
			renameNote = `没有改名：${message(cause)}`;
		} finally {
			renaming = false;
		}
	}

	// 用搜索框补全的接口按“命名空间:”前缀取最常用的标签，切换时取消上一次请求。
	$effect(() => {
		const namespace = selected;
		void version;
		top = [];
		if (!namespace) return;
		const abort = new AbortController();
		const prefix = `${namespace.toLowerCase()}:`;
		suggestTags(prefix, abort.signal, TOP)
			.then((tags) => (top = tags.filter((tag) => tag.tag.startsWith(prefix))))
			.catch((cause) => {
				if (!abort.signal.aborted) error = cause instanceof Error ? cause.message : String(cause);
			});
		return () => abort.abort();
	});

	function value(tag: string) {
		return tag.slice(tag.indexOf(':') + 1);
	}

	function href(tag: string) {
		return `/?${new URLSearchParams({ q: tagTerm(selected ?? '', value(tag)) })}`;
	}
</script>

<div class="adm-head">
	<h1>标签</h1>
	{#if list}
		<span class="adm-sub">
			{count(list.reduce((sum, item) => sum + item.tags, 0))} 个标签 · {list.length} 个命名空间
		</span>
	{/if}
</div>

{#if error}<p class="error">加载失败：{error}</p>{/if}

<form class="adm-card rename" onsubmit={rename}>
	<div class="adm-row">
		<h2>改名或合并</h2>
		<span class="adm-note">写成 命名空间:值。新名字已经有了时两个标签合并；只改大小写也可以。</span>
	</div>
	<div class="fields">
		<input bind:value={from} placeholder="原标签，如 artist:alfa" aria-label="原标签" spellcheck="false" />
		<input bind:value={to} placeholder="新标签，如 artist:alpha" aria-label="新标签" spellcheck="false" />
		<button class="adm-button primary" disabled={renaming || !from.trim() || !to.trim()}>改名</button>
	</div>
	{#if renameNote}<p class="adm-note" role="status">{renameNote}</p>{/if}
</form>

{#if list}
	<div class="layout">
		<section class="adm-scroll" aria-label="命名空间">
			<table class="adm-table">
				<thead>
					<tr><th>命名空间</th><th class="num">标签数</th><th class="num">使用次数</th></tr>
				</thead>
				<tbody>
					{#each list as item (item.namespace)}
						<tr class:active={item.namespace === selected}>
							<td>
								{#if item.namespace}
									<button
										class="pick {namespaceClass(item.namespace)}"
										aria-pressed={item.namespace === selected}
										onclick={() => (selected = item.namespace)}>{item.namespace}</button
									>
								{:else}
									<span class="adm-label">（无命名空间）</span>
								{/if}
							</td>
							<td class="num">{count(item.tags)}</td>
							<td class="num">{count(item.uses)}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</section>

		{#if selected}
			<section class="adm-card top" aria-label="{selected} 下最常用的标签">
				<div class="adm-row">
					<h2 class={namespaceClass(selected)}>{selected}</h2>
					<span class="adm-note">最常用的 {TOP} 个，点一下跳到搜索</span>
				</div>
				{#each top as tag (tag.tag)}
					<a class="bar" href={href(tag.tag)}>
						<span class="name">{value(tag.tag)}</span>
						<span class="adm-meter"><span style:width="{percent(tag.count, top[0].count)}%"></span></span>
						<span class="adm-label">{count(tag.count)}</span>
					</a>
				{/each}
			</section>
		{/if}
	</div>
{/if}

<style>
	.rename {
		display: grid;
		gap: 0.6rem;
		margin-bottom: 1rem;
	}

	.fields {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(min(100%, 14rem), 1fr));
		gap: 0.5rem;
	}

	.fields input {
		min-height: 2.5rem;
		padding: 0 0.7rem;
		border: 1px solid var(--border);
		border-radius: 8px;
		background: var(--sunken);
		color: inherit;
		font: inherit;
	}

	.layout {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(min(100%, 22rem), 1fr));
		gap: 1rem;
		align-items: start;
	}

	tr.active td {
		background: var(--raised);
	}

	.pick {
		padding: 0;
		border: none;
		background: none;
		cursor: pointer;
		text-align: left;
	}

	.pick:hover {
		text-decoration: underline;
	}

	.bar {
		display: grid;
		grid-template-columns: minmax(0, 11rem) minmax(0, 1fr) 4.5rem;
		align-items: center;
		gap: 0.75rem;
		min-height: 1.9rem;
		text-decoration: none;
		font-size: 0.85rem;
	}

	.bar:hover .name {
		color: var(--link);
	}

	.name {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.bar .adm-label {
		text-align: right;
		font-variant-numeric: tabular-nums;
	}
</style>
