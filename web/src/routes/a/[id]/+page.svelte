<script module lang="ts">
	// 书库在历史记录里是否就在这一页之前。从书库点进来时为真；从阅读器按返回回到这里时保持不变。
	let libraryBehind = false;
</script>

<script lang="ts">
	import { afterNavigate } from '$app/navigation';
	import CategoryPicker from '#lib/CategoryPicker.svelte';
	import Icon from '#lib/Icon.svelte';
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

	afterNavigate(({ from, type }) => {
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
		</div>
	</section>

	<div class="categories">
		<CategoryPicker archive={book.id} bind:categories={categoryList} bind:members />
	</div>

	<section class="tags">
		<TagTable rows={tagRows(book)} />
	</section>

	{#if book.summary}
		<section class="summary"><p>{book.summary}</p></section>
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
			'categories categories'
			'tags tags'
			'summary summary'
			'related related';
		gap: 1rem;
		max-width: 64rem;
		margin: 0 auto;
		padding: 0.75rem 1rem 2rem;
	}

	@media (min-width: 720px) {
		main {
			grid-template-columns: 280px minmax(0, 1fr);
			grid-template-rows: auto auto auto 1fr auto;
			grid-template-areas:
				'cover head'
				'cover categories'
				'cover tags'
				'cover summary'
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

	.actions a {
		padding: 0.5rem 0.9rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: var(--surface);
		text-decoration: none;
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
		padding: 0.75rem;
		border-radius: 9px;
		background: var(--surface);
	}

	.summary {
		grid-area: summary;
		line-height: 1.7;
		overflow-wrap: anywhere;
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
