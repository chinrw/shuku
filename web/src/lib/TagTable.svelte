<script lang="ts">
	import type { TagGroup } from './api';
	import { tagTerm } from './query';
	import { namespaceClass, orderGroups } from './tags';

	// 每个命名空间一行，标签可以点进搜索。compact 用在列表视图里。
	let { groups, compact = false }: { groups: TagGroup[]; compact?: boolean } = $props();
</script>

<dl class:compact>
	{#each orderGroups(groups) as group (group.namespace)}
		<dt class={namespaceClass(group.namespace)}>{group.namespace || 'misc'}</dt>
		<dd>
			{#each group.values as value (value)}
				<a class="chip" href="/?q={encodeURIComponent(tagTerm(group.namespace, value))}">{value}</a>
			{/each}
		</dd>
	{/each}
</dl>

<style>
	dl {
		display: grid;
		grid-template-columns: max-content minmax(0, 1fr);
		gap: 0.35rem 0.6rem;
		margin: 0;
	}

	dt {
		padding-top: 0.2rem;
		font-size: 0.8rem;
		font-weight: 600;
		color: var(--muted);
		text-transform: capitalize;
	}

	dd {
		display: flex;
		flex-wrap: wrap;
		gap: 0.3rem;
		margin: 0;
		min-width: 0;
	}

	.chip {
		max-width: 100%;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		padding: 0.15rem 0.5rem;
		border-radius: 4px;
		background: var(--chip);
		color: var(--fg);
		font-size: 0.85rem;
		text-decoration: none;
	}

	.chip:hover {
		color: var(--link);
	}

	.compact {
		gap: 0.2rem 0.45rem;
	}

	.compact dt {
		padding-top: 0.1rem;
		font-size: 0.72rem;
	}

	.compact .chip {
		padding: 0.05rem 0.4rem;
		font-size: 0.75rem;
	}
</style>
