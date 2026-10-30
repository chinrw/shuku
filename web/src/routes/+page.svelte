<script lang="ts">
	import { goto } from '$app/navigation';
	import { search, type ArchiveItem } from '#lib/api.ts';
	import type { Snapshot } from './$types';

	let { data } = $props();

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

	function submit(event: SubmitEvent) {
		event.preventDefault();
		const params = new URLSearchParams();
		if (query.trim()) params.set('q', query.trim());
		if (data.sort !== 'title') params.set('sort', data.sort);
		goto(`/?${params}`);
	}

	function changeSort(sort: string) {
		const params = new URLSearchParams();
		if (data.q) params.set('q', data.q);
		if (sort !== 'title') params.set('sort', sort);
		goto(`/?${params}`);
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
</script>

<header>
	<form onsubmit={submit}>
		<input
			type="search"
			bind:value={query}
			placeholder="搜索：标题，artist:名字，-排除，&quot;精确&quot;"
			enterkeyhint="search"
		/>
		<select value={data.sort} onchange={(event) => changeSort(event.currentTarget.value)}>
			<option value="title">标题</option>
			<option value="date">最新加入</option>
		</select>
	</form>
	{#if data.queryError}
		<p class="error">{data.queryError}</p>
	{:else if data.result}
		<p class="summary">
			共 {data.result.total} 本
			{#if data.result.degraded}
				<span>· 部分通配符还不支持，结果可能偏多</span>
			{/if}
		</p>
	{/if}
</header>

<main>
	{#each items as item (item.id)}
		<a class="card" href="/a/{item.id}">
			<div class="cover">
				<img src={item.cover_url} alt="" loading="lazy" />
				{#if item.is_new}<span class="badge">新</span>{/if}
				{#if item.progress && item.page_count > 0}
					<div class="progress" style:width="{((item.progress.page + 1) / item.page_count) * 100}%"></div>
				{/if}
			</div>
			<div class="title">{item.title}</div>
		</a>
	{/each}
</main>

{#if loadError}
	<p class="error footer">
		加载失败：{loadError}
		<button onclick={loadMore}>重试</button>
	</p>
{:else if next !== null}
	<div class="footer" {@attach sentinel}>{loading ? '加载中…' : ''}</div>
{/if}

<style>
	header {
		position: sticky;
		top: 0;
		z-index: 1;
		padding: 0.75rem 1rem 0.25rem;
		padding-top: max(0.75rem, env(safe-area-inset-top));
		background: var(--bg);
		border-bottom: 1px solid var(--border);
	}

	form {
		display: flex;
		gap: 0.5rem;
	}

	input {
		flex: 1;
		min-width: 0;
		padding: 0.5rem 0.75rem;
		border: 1px solid var(--border);
		border-radius: 0.5rem;
		background: var(--card);
	}

	select {
		border: 1px solid var(--border);
		border-radius: 0.5rem;
		background: var(--card);
	}

	.summary,
	header .error {
		margin: 0.4rem 0;
		font-size: 0.85rem;
		color: var(--muted);
	}

	header .error {
		color: var(--danger);
	}

	main {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
		gap: 1rem 0.75rem;
		padding: 1rem;
	}

	.card {
		text-decoration: none;
		min-width: 0;
	}

	.cover {
		position: relative;
		aspect-ratio: 5 / 7;
		overflow: hidden;
		border-radius: 0.4rem;
		background: var(--card);
	}

	.cover img {
		width: 100%;
		height: 100%;
		object-fit: cover;
	}

	.badge {
		position: absolute;
		top: 0.3rem;
		left: 0.3rem;
		padding: 0 0.35rem;
		border-radius: 0.25rem;
		background: var(--accent);
		color: white;
		font-size: 0.75rem;
	}

	.progress {
		position: absolute;
		left: 0;
		bottom: 0;
		height: 3px;
		background: var(--accent);
	}

	.title {
		margin-top: 0.35rem;
		font-size: 0.85rem;
		line-height: 1.3;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}

	.footer {
		min-height: 3rem;
		padding: 1rem;
		text-align: center;
		color: var(--muted);
	}
</style>
