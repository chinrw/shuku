<script module lang="ts">
	// 书库在历史记录里是否就在这一页之前。从书库点进来时为真；从阅读器按返回回到这里时保持不变。
	let libraryBehind = false;
</script>

<script lang="ts">
	import { afterNavigate } from '$app/navigation';
	import Icon from '#lib/Icon.svelte';
	import TagTable from '#lib/TagTable.svelte';
	import { httpUrl } from '#lib/url.ts';

	let { data } = $props();
	let book = $derived(data.archive);
	let resumePage = $derived(book.progress?.page ?? 0);
	let sourceLink = $derived(book.source_url === null ? null : httpUrl(book.source_url));

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
			{book.page_count} 页 · {formatSize(book.file_size)}<br />
			加入于 {new Date(book.date_added * 1000).toLocaleDateString()}
			{#if book.progress}<br />读到第 {resumePage + 1} 页{/if}
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

	<section class="tags">
		{#if book.tags.length > 0}
			<TagTable groups={book.tags} />
		{:else}
			<p class="meta">没有标签</p>
		{/if}
	</section>

	{#if book.summary || book.source_url}
		<section class="summary">
			{#if book.summary}<p>{book.summary}</p>{/if}
			{#if sourceLink}
				<p class="meta"><a href={sourceLink} target="_blank" rel="noreferrer">来源</a></p>
			{:else if book.source_url}
				<p class="meta">来源：{book.source_url}</p>
			{/if}
		</section>
	{/if}
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
			'tags tags'
			'summary summary';
		gap: 1rem;
		max-width: 64rem;
		margin: 0 auto;
		padding: 0.75rem 1rem 2rem;
	}

	@media (min-width: 720px) {
		main {
			grid-template-columns: 280px minmax(0, 1fr);
			grid-template-rows: auto auto 1fr;
			grid-template-areas:
				'cover head'
				'cover tags'
				'cover summary';
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

	.summary p {
		margin: 0 0 0.5rem;
		white-space: pre-wrap;
	}
</style>
