<script lang="ts">
	import { namespaces as loadNamespaces, type NamespaceCount } from '#lib/admin.ts';
	import { suggestTags, type TagCount } from '#lib/api.ts';
	import { count, percent } from '#lib/format.ts';
	import { tagTerm } from '#lib/query.ts';
	import { namespaceClass } from '#lib/tags.ts';

	const TOP = 30;

	let list = $state<NamespaceCount[] | null>(null);
	let selected = $state<string | null>(null);
	let top = $state<TagCount[]>([]);
	let error = $state<string | null>(null);

	$effect(() => {
		loadNamespaces()
			.then((loaded) => {
				list = loaded;
				selected = loaded.find((item) => item.namespace !== '')?.namespace ?? null;
			})
			.catch((cause) => (error = cause instanceof Error ? cause.message : String(cause)));
	});

	// 用搜索框补全的接口按“命名空间:”前缀取最常用的标签，切换时取消上一次请求。
	$effect(() => {
		const namespace = selected;
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
			{count(list.reduce((sum, item) => sum + item.tags, 0))} 个标签 · {list.length} 个命名空间 · 改名和合并在 M2
		</span>
	{/if}
</div>

{#if error}<p class="error">加载失败：{error}</p>{/if}

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
