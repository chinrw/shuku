<script lang="ts">
	import Icon from './Icon.svelte';
	import { pageNumbers } from './pages';

	// 书库的页码条，和 LRR 一样在列表上下各一条。
	let {
		page,
		pages,
		href
	}: { page: number; pages: number; href: (page: number) => string } = $props();
</script>

{#if pages > 1}
	<nav class="pager" aria-label="翻页">
		{#if page > 1}
			<a href={href(page - 1)} aria-label="上一页"><Icon name="chevron-left" size={16} /></a>
		{/if}
		{#each pageNumbers(page, pages) as number, index (index)}
			{#if number === null}
				<span class="gap">…</span>
			{:else}
				<a href={href(number)} aria-current={number === page ? 'page' : undefined}>{number}</a>
			{/if}
		{/each}
		{#if page < pages}
			<a href={href(page + 1)} aria-label="下一页"><Icon name="chevron-right" size={16} /></a>
		{/if}
	</nav>
{/if}

<style>
	.pager {
		display: flex;
		flex-wrap: wrap;
		justify-content: center;
		gap: 0.3rem;
		margin: 0.75rem 1rem;
	}

	a,
	.gap {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		min-width: 2rem;
		height: 2rem;
		padding: 0 0.4rem;
		border-radius: 6px;
		font-size: 0.85rem;
		font-variant-numeric: tabular-nums;
		text-decoration: none;
	}

	a {
		border: 1px solid var(--chip-border);
		background: var(--chip);
	}

	a:hover {
		color: var(--link);
	}

	a[aria-current='page'] {
		border-color: var(--fg);
		background: var(--fg);
		color: var(--bg);
	}

	.gap {
		color: var(--muted);
	}
</style>
