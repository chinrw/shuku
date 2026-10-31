<script lang="ts">
	import { search, type ArchiveItem } from './api';

	// 详情页底部的一排推荐：同一个标签下的其他本子，最新加入的在前。
	const LIMIT = 13;

	let { label, term, exclude }: { label: string; term: string; exclude: number } = $props();

	let items = $state<ArchiveItem[] | null>(null);
	let total = $state(0);
	let failed = $state<string | null>(null);

	$effect(() => {
		let cancelled = false;
		search(fetch, { q: term, sort: 'date', limit: LIMIT })
			.then((result) => {
				if (cancelled) return;
				items = result.items.filter((item) => item.id !== exclude).slice(0, LIMIT - 1);
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
			<a class="all" href="/?q={encodeURIComponent(term)}&sort=date">全部 {total}</a>
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
