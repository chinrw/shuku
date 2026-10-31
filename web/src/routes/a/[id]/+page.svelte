<script module lang="ts">
	// 书库在历史记录里是否就在这一页之前。从书库点进来时为真；从阅读器按返回回到这里时保持不变。
	let libraryBehind = false;
</script>

<script lang="ts">
	import { afterNavigate, goto, invalidateAll } from '$app/navigation';
	import { onDestroy } from 'svelte';
	import { ApiError, deleteArchive, downloadUrl } from '#lib/api.ts';
	import ArchiveEditor from '#lib/ArchiveEditor.svelte';
	import CategoryPicker from '#lib/CategoryPicker.svelte';
	import Icon from '#lib/Icon.svelte';
	import MetadataFetcher from '#lib/MetadataFetcher.svelte';
	import PageGrid from '#lib/PageGrid.svelte';
	import { relatedQueries } from '#lib/related.ts';
	import RelatedRow from '#lib/RelatedRow.svelte';
	import TagTable from '#lib/TagTable.svelte';
	import { tagRows } from '#lib/tags.ts';

	let { data } = $props();
	let book = $derived(data.archive);
	let resumePage = $derived(book.progress?.page ?? 0);
	// 勾选分类时就地改写，换到另一本书时跟着数据重置。
	let members = $derived(book.categories);
	let categoryList = $derived(data.categories);
	let memberCategories = $derived(
		categoryList.filter((category) => members.includes(category.id))
	);

	// 每次点开编辑是一个新对象，换到另一本书时编辑框自动收起。保存返回前可能已经
	// 换了书或重新点开过编辑，只收起发起保存的那一次。
	let editing = $state.raw<{ id: number } | null>(null);
	let deleting = $state(false);
	let deleteError = $state('');
	let mounted = true;
	onDestroy(() => (mounted = false));

	async function saved(session: { id: number }) {
		await invalidateAll();
		if (editing === session) editing = null;
	}

	async function remove() {
		if (deleting || !confirm(`删除《${book.title}》的记录？阅读进度和分类也一起删除，不能恢复。`)) return;
		const target = book.id;
		deleting = true;
		deleteError = '';
		try {
			await deleteArchive(target);
			// 删除返回前已经去了别的页面时，不把人拽回书库。
			if (mounted && book.id === target) await goto('/', { replaceState: true });
		} catch (cause) {
			if (book.id === target) deleteError = cause instanceof ApiError ? cause.message : '网络错误';
		} finally {
			deleting = false;
		}
	}

	afterNavigate(({ from, to, type }) => {
		// 换了书就结束这次编辑；不清掉的话，回到原书时编辑框会带着旧的保存回调重新出现。
		if (from?.params?.id !== to?.params?.id) editing = null;
		if (from?.route.id === '/') libraryBehind = true;
		else if (!(from?.route.id === '/a/[id]/read' && type === 'popstate')) libraryBehind = false;
	});

	// 用浏览器返回回到书库，书库的快照和滚动位置才会恢复。
	function back(event: MouseEvent) {
		if (!libraryBehind) return;
		event.preventDefault();
		history.back();
	}

	function formatSize(bytes: number) {
		if (bytes >= 1 << 30) return `${(bytes / (1 << 30)).toFixed(1)} GiB`;
		if (bytes >= 1 << 20) return `${(bytes / (1 << 20)).toFixed(1)} MiB`;
		return `${Math.ceil(bytes / 1024)} KiB`;
	}
</script>

<svelte:head>
	<title>{book.title} · Shuku</title>
</svelte:head>

<nav>
	<a class="icon-button" href="/" onclick={back} aria-label="返回书库"><Icon name="back" /></a>
</nav>

<main>
	<a class="cover" href="/a/{book.id}/read?page={resumePage}">
		<img src={book.cover_url} alt="" />
	</a>

	<section class="head">
		<h1>{book.title}</h1>
		<p class="meta">
			{book.page_count} 页 · {formatSize(book.file_size)}
			{#if book.progress}<br />读到第 {resumePage + 1} 页{/if}
			{#if memberCategories.length}
				<br />分类：{#each memberCategories as category, index (category.id)}{#if index}、{/if}<a
						href="/?c={category.id}">{category.name}</a
					>{/each}
			{/if}
		</p>
		<div class="actions">
			{#if book.progress}
				<a class="primary" href="/a/{book.id}/read?page={resumePage}">继续阅读</a>
				<a href="/a/{book.id}/read?page=0">从头</a>
			{:else}
				<a class="primary" href="/a/{book.id}/read?page=0">开始阅读</a>
			{/if}
			<a href={downloadUrl(book.id)} download>下载</a>
			{#if editing?.id !== book.id}
				<button onclick={() => (editing = { id: book.id })}>编辑</button>
			{/if}
		</div>
	</section>

	{#if book.missing}
		<section class="missing" role="status">
			<p>文件已经不在内容目录里了：<span class="path">{book.path}</span></p>
			<p>文件改名或移动后，下次扫描会认出它并保留这条记录。确实删掉了的话，可以删除记录。</p>
			<button disabled={deleting} onclick={remove}>删除这条记录</button>
			{#if deleteError}<p class="error" role="alert">没有删除：{deleteError}</p>{/if}
		</section>
	{/if}

	<div class="categories">
		<CategoryPicker archive={book.id} bind:categories={categoryList} bind:members />
		<!-- 换书时重建，上一本填的画廊地址不会带到这一本。 -->
		{#key book.id}<MetadataFetcher archive={book.id} />{/key}
	</div>

	{#if editing?.id === book.id}
		{@const session = editing}
		<section class="tags">
			<ArchiveEditor {book} onsaved={() => saved(session)} oncancel={() => (editing = null)} />
		</section>
	{:else}
		<section class="tags table">
			<TagTable rows={tagRows(book)} />
		</section>

		{#if book.summary}
			<section class="summary"><p>{book.summary}</p></section>
		{/if}
	{/if}

	{#if data.pages.length}
		<div class="pages">
			<PageGrid archive={book.id} pages={data.pages} current={book.progress ? resumePage : null} />
		</div>
	{/if}

	<div class="related">
		{#each relatedQueries(book.tags) as query (book.id + query.term)}
			<RelatedRow label={query.label} term={query.term} exclude={book.id} />
		{/each}
	</div>
</main>

<style>
	nav {
		padding: 0.6rem 1rem 0;
		padding-top: max(0.6rem, env(safe-area-inset-top));
	}

	/* 手机上封面和标题并排，标签和摘要占满宽度；宽屏上封面固定在左列。 */
	main {
		display: grid;
		grid-template-columns: minmax(0, 38%) minmax(0, 1fr);
		grid-template-areas:
			'cover head'
			'missing missing'
			'categories categories'
			'tags tags'
			'summary summary'
			'pages pages'
			'related related';
		gap: 1rem;
		max-width: 64rem;
		margin: 0 auto;
		padding: 0.75rem 1rem 2rem;
	}

	@media (min-width: 720px) {
		main {
			grid-template-columns: 280px minmax(0, 1fr);
			grid-template-rows: auto auto auto auto 1fr auto auto;
			grid-template-areas:
				'cover head'
				'cover missing'
				'cover categories'
				'cover tags'
				'cover summary'
				'pages pages'
				'related related';
			column-gap: 1.75rem;
		}
	}

	.cover {
		grid-area: cover;
	}

	.cover img {
		width: 100%;
		border-radius: 8px;
		background: var(--chip);
	}

	.head {
		grid-area: head;
		min-width: 0;
	}

	h1 {
		margin: 0 0 0.5rem;
		font-size: 1.15rem;
		line-height: 1.4;
		overflow-wrap: anywhere;
	}

	@media (min-width: 720px) {
		h1 {
			font-size: 1.4rem;
		}
	}

	.meta {
		margin: 0;
		color: var(--muted);
		font-size: 0.85rem;
		line-height: 1.6;
	}

	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
		margin-top: 0.9rem;
	}

	.actions a,
	.actions button,
	.missing button {
		padding: 0.5rem 0.9rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: var(--surface);
		color: inherit;
		font: inherit;
		text-decoration: none;
		cursor: pointer;
	}

	.missing {
		grid-area: missing;
		display: grid;
		justify-items: start;
		gap: 0.5rem;
		padding: 0.75rem;
		border: 1px solid var(--danger);
		border-radius: 9px;
		font-size: 0.85rem;
		line-height: 1.6;
	}

	.missing p {
		margin: 0;
	}

	.missing .path {
		font-family: ui-monospace, monospace;
		overflow-wrap: anywhere;
	}

	.missing button {
		border-color: var(--danger);
		color: var(--danger);
	}

	.missing button:disabled {
		opacity: 0.6;
		cursor: default;
	}

	.actions a.primary {
		border-color: var(--accent);
		background: var(--accent);
		color: white;
	}

	.categories {
		grid-area: categories;
		min-width: 0;
	}

	.tags {
		grid-area: tags;
		min-width: 0;
	}

	.tags.table {
		padding: 0.75rem;
		border-radius: 9px;
		background: var(--surface);
	}

	.summary {
		grid-area: summary;
		line-height: 1.7;
		overflow-wrap: anywhere;
	}

	.pages {
		grid-area: pages;
		min-width: 0;
		margin-top: 0.5rem;
	}

	.related {
		grid-area: related;
		display: grid;
		gap: 1.25rem;
		min-width: 0;
		margin-top: 0.5rem;
	}

	.summary p {
		margin: 0 0 0.5rem;
		white-space: pre-wrap;
	}
</style>
