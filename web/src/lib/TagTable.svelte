<script lang="ts">
	import { namespaceClass, type TagRow } from './tags';

	// LRR 的标签表：左列是命名空间，右列是可点的标签。compact 用在列表视图里。
	let { rows, compact = false }: { rows: TagRow[]; compact?: boolean } = $props();
</script>

<dl class:compact>
	{#each rows as row (row.namespace)}
		<dt class={namespaceClass(row.namespace)}>{row.label}:</dt>
		<dd>
			{#each row.values as value, index (index)}
				{#if value.href === null}
					<span class="chip plain">{value.text}</span>
				{:else if value.external}
					<a class="chip" href={value.href} target="_blank" rel="noreferrer">{value.text}</a>
				{:else}
					<a class="chip" href={value.href}>{value.text}</a>
				{/if}
			{/each}
		</dd>
	{/each}
</dl>

<style>
	dl {
		display: grid;
		grid-template-columns: max-content minmax(0, 1fr);
		gap: 0.3rem 0.6rem;
		margin: 0;
	}

	dt {
		padding-top: 0.2rem;
		font-size: 0.82rem;
		font-weight: 600;
		color: var(--muted);
	}

	dd {
		display: flex;
		flex-wrap: wrap;
		gap: 0.3rem;
		margin: 0;
		min-width: 0;
	}

	/* LRR 的 div.gt：带边框的小块。 */
	.chip {
		max-width: 100%;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		padding: 0.1rem 0.45rem;
		border: 1px solid var(--chip-border);
		border-radius: 5px;
		background: var(--chip);
		color: var(--fg);
		font-size: 0.85rem;
		text-decoration: none;
	}

	a.chip:hover {
		color: var(--link);
	}

	.chip.plain {
		border-color: transparent;
	}

	.compact {
		gap: 0.2rem 0.45rem;
	}

	.compact dt {
		padding-top: 0.1rem;
		font-size: 0.72rem;
	}

	.compact .chip {
		padding: 0 0.35rem;
		font-size: 0.75rem;
	}
</style>
