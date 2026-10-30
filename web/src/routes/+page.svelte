<script lang="ts">
	import { goto } from '$app/navigation';
	import { search, type ArchiveItem } from '#lib/api.ts';
	import Icon from '#lib/Icon.svelte';
	import { settings } from '#lib/settings.svelte.ts';
	import TagLine from '#lib/TagLine.svelte';
	import TagPopover from '#lib/TagPopover.svelte';
	import TagTable from '#lib/TagTable.svelte';
	import { tagRows } from '#lib/tags.ts';
	import type { Snapshot } from './$types';

	let { data } = $props();
	const prefs = settings();

	// 翻到后面的页时在这里追加；搜索条件变化后 data 更新，两者随之重置。
	let items = $derived<ArchiveItem[]>(data.result?.items ?? []);
	let next = $derived(data.result?.next_after ?? null);
	let query = $derived(data.q);
	let loading = $state(false);
	let loadError = $state<string | null>(null);

	// 返回列表时恢复已经加载的条目，浏览器才能把滚动位置还原到原处。
	export const snapshot: Snapshot<{ items: ArchiveItem[]; next: number | null }> = {
		capture: () => ({ items, next }),
		restore: (saved) => {
			items = saved.items;
			next = saved.next;
		}
	};

	function navigate(q: string, sort: string) {
		const params = new URLSearchParams();
		if (q) params.set('q', q);
		if (sort !== 'title') params.set('sort', sort);
		goto(params.size ? `/?${params}` : '/');
	}

	function submit(event: SubmitEvent) {
		event.preventDefault();
		navigate(query.trim(), data.sort);
	}

	function toggleView() {
		prefs.view = prefs.view === 'grid' ? 'list' : 'grid';
		prefs.save();
	}

	async function loadMore() {
		if (loading || next === null) return;
		loading = true;
		loadError = null;
		try {
			const result = await search(fetch, { q: data.q, sort: data.sort, after: next });
			items = [...items, ...result.items];
			next = result.next_after;
		} catch (error) {
			loadError = error instanceof Error ? error.message : String(error);
		} finally {
			loading = false;
		}
	}

	// 条目数变化后重新观察，加载完一页后哨兵仍在视口内时会继续加载下一页。
	function sentinel(node: HTMLElement) {
		void items.length;
		const observer = new IntersectionObserver(
			(entries) => {
				if (entries[0].isIntersecting && loadError === null) loadMore();
			},
			{ rootMargin: '800px' }
		);
		observer.observe(node);
		return () => observer.disconnect();
	}

	// 标签弹窗：鼠标悬停时打开，移开后稍等再关，让鼠标能移进弹窗里点标签；触屏点一下打开。
	const HOVER_DELAY = 150;
	let tagView = $state<{ item: ArchiveItem; anchor: DOMRect; sheet: boolean } | null>(null);
	let hoverTimer: ReturnType<typeof setTimeout> | undefined;

	function openTags(item: ArchiveItem, target: HTMLElement) {
		clearTimeout(hoverTimer);
		tagView = { item, anchor: target.getBoundingClientRect(), sheet: window.innerWidth < 600 };
	}

	function closeTags() {
		clearTimeout(hoverTimer);
		tagView = null;
	}

	function hoverIn(event: PointerEvent, item?: ArchiveItem) {
		if (event.pointerType !== 'mouse') return;
		clearTimeout(hoverTimer);
		const target = event.currentTarget as HTMLElement;
		if (item) hoverTimer = setTimeout(() => openTags(item, target), HOVER_DELAY);
	}

	function hoverOut(event: PointerEvent) {
		if (event.pointerType !== 'mouse') return;
		clearTimeout(hoverTimer);
		hoverTimer = setTimeout(() => (tagView = null), HOVER_DELAY);
	}

	function tapTags(event: MouseEvent, item: ArchiveItem) {
		const target = event.currentTarget as HTMLElement;
		if (tagView?.item.id === item.id && !tagView.sheet) closeTags();
		else openTags(item, target);
	}

	function progressText(item: ArchiveItem) {
		return item.progress ? `读到 ${item.progress.page + 1}` : item.is_new ? '新' : '';
	}
</script>

<header>
	<a class="brand" href="/" onclick={() => (query = '')}>书库</a>
	<form onsubmit={submit} role="search">
		<Icon name="search" size={16} />
		<input
			type="search"
			bind:value={query}
			placeholder="标题或标签，如 artist:名字，-排除"
			enterkeyhint="search"
			autocapitalize="off"
			autocomplete="off"
			spellcheck="false"
		/>
	</form>
	<div class="controls">
		<select
			value={data.sort}
			onchange={(event) => navigate(data.q, event.currentTarget.value)}
			aria-label="排序"
		>
			<option value="title">标题</option>
			<option value="date">最新加入</option>
		</select>
		<button
			class="icon-button"
			onclick={toggleView}
			aria-label={prefs.view === 'grid' ? '列表视图' : '封面视图'}
		>
			<Icon name={prefs.view === 'grid' ? 'list' : 'grid'} size={18} />
		</button>
	</div>
</header>

{#if data.queryError}
	<p class="status error">{data.queryError}</p>
{:else if data.result}
	<p class="status">
		共 {data.result.total} 本{#if data.q}，搜索“{data.q}”{/if}
		{#if data.result.degraded}<span>· 部分通配符还不支持，结果可能偏多</span>{/if}
	</p>
{/if}

{#if prefs.view === 'grid'}
	<main class="grid">
		{#each items as item (item.id)}
			<div class="card">
				<a class="title" href="/a/{item.id}" title={item.title}>{item.title}</a>
				<a class="cover" href="/a/{item.id}">
					<img src={item.cover_url} alt="" loading="lazy" />
					{#if item.is_new && !item.progress}<span class="badge new">新</span>{/if}
					<span class="badge pages">{item.page_count}P</span>
					{#if item.progress && item.page_count > 0}
						<div
							class="progress"
							style:width="{((item.progress.page + 1) / item.page_count) * 100}%"
						></div>
					{/if}
				</a>
				{#if item.tags.length > 0}
					<button
						class="tags"
						aria-label="查看全部标签"
						aria-expanded={tagView?.item.id === item.id}
						onclick={(event) => tapTags(event, item)}
						onpointerenter={(event) => hoverIn(event, item)}
						onpointerleave={hoverOut}
					>
						<TagLine groups={item.tags} />
					</button>
				{/if}
			</div>
		{/each}
	</main>
{:else}
	<main class="list">
		{#each items as item (item.id)}
			<article class="row">
				<a class="thumb" href="/a/{item.id}">
					<img src={item.cover_url} alt="" loading="lazy" />
				</a>
				<div class="body">
					<a class="title" href="/a/{item.id}">{item.title}</a>
					<div class="meta">
						{item.page_count} 页
						{#if progressText(item)}· <span class:fresh={item.is_new && !item.progress}>{progressText(item)}</span>{/if}
					</div>
					<!-- 列表里每本都显示加入日期和来源太占地方，这两行只放在弹窗和详情页里。 -->
					<TagTable
						rows={tagRows(item).filter(
							(row) => row.namespace !== 'date_added' && row.namespace !== 'source'
						)}
						compact
					/>
				</div>
			</article>
		{/each}
	</main>
{/if}

{#if tagView}
	<TagPopover
		item={tagView.item}
		anchor={tagView.anchor}
		sheet={tagView.sheet}
		onclose={closeTags}
		onpointerenter={(event) => hoverIn(event)}
		onpointerleave={hoverOut}
	/>
{/if}

<svelte:window
	onkeydown={(event) => event.key === 'Escape' && closeTags()}
	onscroll={() => tagView && !tagView.sheet && closeTags()}
	onpointerdown={(event) => {
		// 点在浮层和标签行以外的地方时关闭；底部面板由遮罩处理。
		const target = event.target as Element;
		if (tagView && !tagView.sheet && !target.closest('.popover, .tags')) closeTags();
	}}
/>

{#if loadError}
	<p class="footer error">
		加载失败：{loadError}
		<button class="icon-button" onclick={loadMore}>重试</button>
	</p>
{:else if next !== null}
	<div class="footer" {@attach sentinel}>{loading ? '加载中…' : ''}</div>
{:else if items.length > 0}
	<div class="footer">没有更多了</div>
{/if}

<style>
	header {
		position: sticky;
		top: 0;
		z-index: 2;
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.5rem 0.75rem;
		padding: 0.6rem 1rem;
		padding-top: max(0.6rem, env(safe-area-inset-top));
		background: color-mix(in srgb, var(--bg) 92%, transparent);
		backdrop-filter: blur(8px);
		-webkit-backdrop-filter: blur(8px);
		border-bottom: 1px solid var(--border);
	}

	.brand {
		font-weight: 700;
		font-size: 1.1rem;
		text-decoration: none;
	}

	form {
		flex: 1;
		display: flex;
		align-items: center;
		gap: 0.4rem;
		min-width: 0;
		max-width: 44rem;
		height: 2.25rem;
		padding: 0 0.7rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: var(--surface);
		color: var(--muted);
	}

	input {
		flex: 1;
		min-width: 0;
		border: none;
		outline: none;
		background: transparent;
		color: var(--fg);
	}

	.controls {
		display: flex;
		gap: 0.5rem;
		margin-left: auto;
	}

	select {
		height: 2.25rem;
		padding: 0 0.5rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: var(--surface);
	}

	/* 手机上搜索框单独占一行，品牌和按钮在上面一行。 */
	@media (max-width: 599px) {
		form {
			order: 3;
			flex-basis: 100%;
			max-width: none;
		}
	}

	.status {
		margin: 0.6rem 1rem 0;
		font-size: 0.85rem;
		color: var(--muted);
	}

	.grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(155px, 1fr));
		gap: 0.6rem;
		padding: 0.75rem;
	}

	/* 宽屏上限 1600px，常见的 1440 到 1920 宽屏都是一排五本。 */
	@media (min-width: 1024px) {
		.grid {
			grid-template-columns: repeat(auto-fill, minmax(250px, 1fr));
			gap: 1rem;
			max-width: 1600px;
			margin: 0 auto;
			padding: 1.25rem;
		}
	}

	.card {
		display: flex;
		flex-direction: column;
		gap: 0.4rem;
		min-width: 0;
		padding: 0.45rem 0.45rem 0.55rem;
		border-radius: 9px;
		background: var(--surface);
		text-decoration: none;
	}

	.card .title {
		text-decoration: none;
		min-height: 2.5em;
		font-size: 0.85rem;
		font-weight: 600;
		line-height: 1.25;
		text-align: center;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}

	.cover {
		display: block;
		position: relative;
		aspect-ratio: 5 / 7;
		overflow: hidden;
		border-radius: 6px;
		background: var(--chip);
	}

	.tags {
		-webkit-tap-highlight-color: transparent;
		padding: 0;
		border: none;
		background: none;
		text-align: left;
		cursor: pointer;
	}

	.tags:hover :global(.tagline) {
		color: var(--fg);
	}

	.cover img {
		display: block;
		width: 100%;
		height: 100%;
		object-fit: cover;
		object-position: center top;
	}

	.badge {
		position: absolute;
		padding: 0.05rem 0.35rem;
		border-radius: 4px;
		font-size: 0.72rem;
		line-height: 1.4;
	}

	.badge.new {
		top: 0.3rem;
		left: 0.3rem;
		background: var(--new);
		color: white;
	}

	.badge.pages {
		right: 0.3rem;
		bottom: 0.45rem;
		background: rgb(0 0 0 / 0.6);
		color: #eee;
	}

	.progress {
		position: absolute;
		left: 0;
		bottom: 0;
		height: 4px;
		background: var(--accent);
	}

	.list {
		display: grid;
		gap: 0.5rem;
		max-width: 72rem;
		margin: 0 auto;
		padding: 0.75rem;
	}

	@media (min-width: 1280px) {
		.list {
			grid-template-columns: 1fr 1fr;
			max-width: none;
		}
	}

	.row {
		display: flex;
		gap: 0.75rem;
		min-width: 0;
		padding: 0.5rem;
		border-radius: 9px;
		background: var(--surface);
	}

	.thumb {
		flex: none;
		width: 84px;
	}

	.thumb img {
		width: 100%;
		aspect-ratio: 5 / 7;
		object-fit: cover;
		object-position: center top;
		border-radius: 5px;
		background: var(--chip);
	}

	.body {
		display: flex;
		flex-direction: column;
		gap: 0.3rem;
		min-width: 0;
	}

	.body .title {
		font-weight: 600;
		line-height: 1.3;
		text-decoration: none;
	}

	.meta {
		font-size: 0.8rem;
		color: var(--muted);
	}

	.fresh {
		color: var(--new);
	}

	.footer {
		min-height: 3rem;
		padding: 1rem;
		text-align: center;
		color: var(--muted);
		font-size: 0.85rem;
	}
</style>
