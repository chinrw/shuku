<script lang="ts">
	import { search, similar, type ArchiveItem } from './api';

	// 详情页底部给 archive 这本书的一排推荐，不含它自己。给 term 时是同一个标签下的
	// 其他本子，最新加入的在前，带“全部”链接；不给时是按标签相似度排的相似作品。
	const LIMIT = 13;

	let { label, term, archive }: { label: string; term?: string; archive: number } = $props();

	let items = $state<ArchiveItem[] | null>(null);
	let total = $state(0);
	let failed = $state<string | null>(null);

	$effect(() => {
		let cancelled = false;
		const load =
			term === undefined
				? similar(fetch, archive, LIMIT).then((items) => ({ items, total: 0 }))
				: search(fetch, { q: term, sort: 'date', limit: LIMIT });
		load
			.then((result) => {
				if (cancelled) return;
				items = result.items.filter((item) => item.id !== archive).slice(0, LIMIT - 1);
				total = result.total;
			})
			.catch((error) => {
				if (!cancelled) failed = error instanceof Error ? error.message : String(error);
			});
		return () => {
			cancelled = true;
		};
	});
</script>

{#if failed}
	<section>
		<h2>{label}</h2>
		<p class="error">加载失败：{failed}</p>
	</section>
{:else if items && items.length > 0}
	<section>
		<h2>
			{label}
			{#if term !== undefined}
				<a class="all" href="/?q={encodeURIComponent(term)}&sort=date">全部 {total}</a>
			{/if}
		</h2>
		<div class="strip">
			{#each items as item (item.id)}
				<a class="book" href="/a/{item.id}/read">
					<img src={item.cover_url} alt="" loading="lazy" />
					<span>{item.title}</span>
				</a>
			{/each}
		</div>
	</section>
{/if}

<style>
	h2 {
		display: flex;
		align-items: baseline;
		gap: 0.75rem;
		margin: 0 0 0.6rem;
		font-size: 0.95rem;
	}

	.all {
		margin-left: auto;
		font-size: 0.8rem;
		font-weight: normal;
		color: var(--link);
		text-decoration: none;
	}

	.strip {
		display: grid;
		grid-auto-flow: column;
		grid-auto-columns: 7.5rem;
		gap: 0.6rem;
		overflow-x: auto;
		padding-bottom: 0.4rem;
		scroll-snap-type: x proximity;
	}

	.book {
		scroll-snap-align: start;
		text-decoration: none;
		font-size: 0.78rem;
		line-height: 1.3;
	}

	.book img {
		display: block;
		width: 100%;
		aspect-ratio: 5 / 7;
		object-fit: cover;
		object-position: center top;
		border-radius: 6px;
		background: var(--chip);
	}

	.book span {
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
		margin-top: 0.3rem;
	}
</style>
