<script lang="ts">
	import { afterNavigate } from '$app/navigation';
	import { tagTerm } from '#lib/query.ts';
	import { httpUrl } from '#lib/url.ts';

	let { data } = $props();
	let book = $derived(data.archive);
	let resumePage = $derived(book.progress?.page ?? 0);
	let sourceLink = $derived(book.source_url === null ? null : httpUrl(book.source_url));

	// 从书库点进来时用浏览器返回，书库的快照和滚动位置才会恢复。
	let fromLibrary = false;
	afterNavigate(({ from }) => {
		fromLibrary = from?.url.pathname === '/';
	});

	function back(event: MouseEvent) {
		if (!fromLibrary) return;
		event.preventDefault();
		history.back();
	}

	function formatSize(bytes: number) {
		return bytes >= 1 << 30
			? `${(bytes / (1 << 30)).toFixed(1)} GiB`
			: `${(bytes / (1 << 20)).toFixed(1)} MiB`;
	}
</script>

<svelte:head>
	<title>{book.title} · Shuku</title>
</svelte:head>

<nav><a href="/" onclick={back}>‹ 书库</a></nav>

<main>
	<img class="cover" src={book.cover_url} alt="" />
	<section>
		<h1>{book.title}</h1>
		<p class="meta">
			{book.page_count} 页 · {formatSize(book.file_size)} · 加入于
			{new Date(book.date_added * 1000).toLocaleDateString()}
		</p>
		<div class="actions">
			{#if book.progress}
				<a class="primary" href="/a/{book.id}/read?page={resumePage}">
					继续阅读（第 {resumePage + 1} 页）
				</a>
				<a href="/a/{book.id}/read?page=0">从头阅读</a>
			{:else}
				<a class="primary" href="/a/{book.id}/read?page=0">开始阅读</a>
			{/if}
		</div>
		{#if book.tags.length > 0}
			<dl>
				{#each book.tags as group (group.namespace)}
					<dt>{group.namespace || '标签'}</dt>
					<dd>
						{#each group.values as value (value)}
							<a class="tag" href="/?q={encodeURIComponent(tagTerm(group.namespace, value))}">{value}</a>
						{/each}
					</dd>
				{/each}
			</dl>
		{/if}
		{#if book.summary}
			<p class="summary">{book.summary}</p>
		{/if}
		{#if sourceLink}
			<p class="meta"><a href={sourceLink} target="_blank" rel="noreferrer">来源</a></p>
		{:else if book.source_url}
			<p class="meta">来源：{book.source_url}</p>
		{/if}
	</section>
</main>

<style>
	nav {
		padding: 0.75rem 1rem;
		padding-top: max(0.75rem, env(safe-area-inset-top));
	}

	nav a {
		text-decoration: none;
		color: var(--muted);
	}

	main {
		display: grid;
		grid-template-columns: minmax(0, 260px) 1fr;
		gap: 1.5rem;
		padding: 0 1rem 2rem;
	}

	@media (max-width: 640px) {
		main {
			grid-template-columns: 1fr;
		}

		.cover {
			max-width: 220px;
		}
	}

	.cover {
		width: 100%;
		border-radius: 0.4rem;
	}

	h1 {
		margin: 0 0 0.4rem;
		font-size: 1.3rem;
		line-height: 1.4;
	}

	.meta {
		color: var(--muted);
		font-size: 0.9rem;
	}

	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
		margin: 1rem 0;
	}

	.actions a {
		padding: 0.5rem 1rem;
		border: 1px solid var(--border);
		border-radius: 0.5rem;
		text-decoration: none;
	}

	.actions a.primary {
		background: var(--accent);
		border-color: var(--accent);
		color: white;
	}

	dl {
		display: grid;
		grid-template-columns: max-content 1fr;
		gap: 0.4rem 0.75rem;
		margin: 0;
	}

	dt {
		color: var(--muted);
		font-size: 0.85rem;
		padding-top: 0.2rem;
	}

	dd {
		display: flex;
		flex-wrap: wrap;
		gap: 0.3rem;
		margin: 0;
	}

	.tag {
		padding: 0.1rem 0.5rem;
		border: 1px solid var(--border);
		border-radius: 1rem;
		background: var(--card);
		font-size: 0.85rem;
		text-decoration: none;
	}

	.summary {
		white-space: pre-wrap;
		line-height: 1.6;
	}
</style>
