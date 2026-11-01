<script lang="ts">
	import { untrack } from 'svelte';
	import { archive as loadArchive, downloadUrl, type ArchiveItem, type Category } from '#lib/api.ts';
	import BookmarkButton from '#lib/BookmarkButton.svelte';
	import CategoryPicker from '#lib/CategoryPicker.svelte';
	import FinishedButton from '#lib/FinishedButton.svelte';
	import Icon from '#lib/Icon.svelte';
	import { fade, slide } from '#lib/motion.ts';
	import { artistQueries } from '#lib/related.ts';
	import RelatedRow from '#lib/RelatedRow.svelte';
	import TagTable from '#lib/TagTable.svelte';
	import { tagRows } from '#lib/tags.ts';

	// 书库里点标题或标签行，从右侧滑出这本书的详情。电脑上不加遮罩，
	// 抽屉开着也能点别的书换一本；手机上抽屉占满屏幕宽，点遮罩关闭。
	let {
		item,
		categories = $bindable(),
		onclose
	}: { item: ArchiveItem; categories: Category[]; onclose: () => void } = $props();

	// 关掉抽屉时父页面先把 item 置空再销毁这个组件，分类勾选的回调晚到时还会读归档 ID，
	// 所以下面都用最后显示的那一本。
	let lastItem = untrack(() => item);
	$effect.pre(() => {
		if (item) lastItem = item;
	});
	let book = $derived(item ?? lastItem);

	// 分类成员和简介只在详情接口里有，换书时重新取。
	let detail = $state<{ id: number; members: number[]; summary: string } | null>(null);
	let detailError = $state('');
	// 换书或关掉抽屉后，晚到的回复丢掉；关掉后 item 已经不可用，回调里不能再读它。
	$effect(() => {
		const id = book.id;
		let current = true;
		detail = null;
		detailError = '';
		loadArchive(fetch, id).then(
			(book) => {
				if (current) detail = { id, members: book.categories, summary: book.summary };
			},
			(error) => {
				if (current) detailError = error instanceof Error ? error.message : String(error);
			}
		);
		return () => {
			current = false;
		};
	});

	// 列表项的进度来自快照，可能比服务端旧，所以继续阅读的链接不带页码，由阅读器取最新进度。
	let resume = $derived(book.progress?.page ?? 0);
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="scrim" onclick={onclose} in:fade out:fade></div>
<aside
	class="drawer"
	aria-label="{book.title} 的详情"
	in:slide={{ from: 'right' }}
	out:slide={{ from: 'right' }}
>
	<div class="head">
		<button class="icon-button" onclick={onclose} aria-label="关闭详情"><Icon name="close" /></button>
	</div>
	<div class="book">
		<a class="cover" href="/a/{book.id}/read">
			<img src={book.cover_url} alt="" />
		</a>
		<div class="info">
			<h2>{book.title}</h2>
			<p class="meta">
				{book.page_count} 页
				{#if book.progress}<br />读到第 {resume + 1} 页{/if}
			</p>
		</div>
	</div>
	<div class="actions">
		<a class="primary" href="/a/{book.id}/read">{book.progress ? '继续阅读' : '开始阅读'}</a>
		{#if detail?.id === book.id}
			<BookmarkButton archive={book.id} members={detail.members} bind:categories />
		{/if}
		{#if book.progress}<a href="/a/{book.id}/read?page=0">从头</a>{/if}
		<a href="/a/{book.id}/read?panel=detail">详情</a>
		<FinishedButton archive={book.id} progress={book.progress} />
		<a href={downloadUrl(book.id)} download>下载</a>
		<a href="/a/{book.id}?edit">编辑</a>
	</div>
	<div class="tags">
		<TagTable rows={tagRows(book)} saveable />
	</div>
	{#if detail?.id === book.id}
		<CategoryPicker archive={book.id} bind:categories members={detail.members} />
		{#if detail.summary}<p class="summary">{detail.summary}</p>{/if}
	{:else if detailError}
		<p class="error">分类和简介没有加载出来：{detailError}</p>
	{/if}
	{#key book.id}
		{#each artistQueries(book.tags) as query (query.term)}
			<RelatedRow label={query.label} term={query.term} archive={book.id} />
		{/each}
		<RelatedRow label="相似作品" archive={book.id} />
	{/key}
</aside>

<style>
	.drawer {
		position: fixed;
		top: 0;
		right: 0;
		bottom: 0;
		z-index: 20;
		display: flex;
		flex-direction: column;
		gap: 1rem;
		width: min(400px, 100%);
		overflow-y: auto;
		padding: 0.75rem 1.25rem 1.75rem;
		padding-top: max(0.75rem, env(safe-area-inset-top));
		background: var(--raised);
		box-shadow: -16px 0 40px rgb(0 0 0 / 0.35);
	}

	.scrim {
		position: fixed;
		inset: 0;
		z-index: 19;
		background: rgb(0 0 0 / 0.45);
	}

	@media (min-width: 600px) {
		.scrim {
			display: none;
		}
	}

	.head {
		display: flex;
		justify-content: flex-end;
	}

	.book {
		display: flex;
		gap: 0.9rem;
	}

	.cover {
		flex: none;
		width: 120px;
	}

	.cover img {
		display: block;
		width: 100%;
		border-radius: 6px;
		background: var(--chip);
	}

	.info {
		min-width: 0;
	}

	h2 {
		margin: 0 0 0.5rem;
		font-size: 1rem;
		line-height: 1.4;
		overflow-wrap: anywhere;
	}

	.meta {
		margin: 0;
		color: var(--muted);
		font-size: 0.8rem;
		line-height: 1.6;
	}

	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
	}

	.actions a,
	.actions :global(.finished-button) {
		padding: 0.55rem 0.9rem;
		border: 1px solid var(--chip-border);
		border-radius: 8px;
		background: transparent;
		color: inherit;
		font: inherit;
		text-decoration: none;
		white-space: nowrap;
		cursor: pointer;
	}

	/* 手机上“开始阅读”被挤成两行、别的按钮跟着变高；收窄内边距让一排放得下，放不下时整个按钮换行。 */
	@media (max-width: 480px) {
		.actions a,
		.actions :global(.finished-button) {
			padding: 0.55rem 0.7rem;
		}
	}

	.actions a.primary {
		flex: 1;
		border-color: var(--accent);
		background: var(--accent);
		color: var(--on-accent);
		font-weight: 600;
		text-align: center;
	}

	.tags {
		padding: 0.75rem;
		border-radius: 9px;
		background: var(--surface);
	}

	.summary {
		margin: 0;
		line-height: 1.7;
		white-space: pre-wrap;
		overflow-wrap: anywhere;
	}

	.error {
		margin: 0;
		color: var(--danger);
		font-size: 0.85rem;
	}
</style>
