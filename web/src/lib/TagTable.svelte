<script lang="ts">
	import Icon from './Icon.svelte';
	import { savedTagStore } from './savedTags.svelte';
	import { namespaceClass, type TagRow } from './tags';

	// LRR 的标签表：左列是命名空间，右列是可点的标签。compact 用在列表视图里。
	// saveable 时每个标签后面有书签图标，点一下收藏到书库的筛选行或取消。
	let {
		rows,
		compact = false,
		saveable = false
	}: { rows: TagRow[]; compact?: boolean; saveable?: boolean } = $props();

	const saved = savedTagStore();
	$effect(() => {
		if (saveable) saved.ensure();
	});
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
				{#if saveable && value.tag}
					{@const on = saved.has(value.tag)}
					<button
						class="save"
						class:on
						aria-pressed={on}
						aria-label={on ? `取消收藏标签 ${value.text}` : `收藏标签 ${value.text}`}
						title={saved.error ? `没有保存：${saved.error}` : on ? '已收藏，点一下取消' : '收藏这个标签'}
						disabled={saved.isPending(value.tag)}
						onclick={() => value.tag && saved.toggle(value.tag)}
					>
						<Icon name="bookmark" size={12} filled={on} />
					</button>
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

	/* 收藏按钮紧跟在标签后面，平时淡一些，收藏了用强调色。 */
	.save {
		display: inline-flex;
		align-items: center;
		margin-left: -0.2rem;
		padding: 0.1rem;
		border: none;
		background: transparent;
		color: var(--faint, #7c8087);
		cursor: pointer;
	}

	/* 手机上左边那列命名空间太占宽度：改成命名空间一行、标签在下面一行。 */
	@media (max-width: 599px) {
		dl:not(.compact) {
			grid-template-columns: minmax(0, 1fr);
			gap: 0.1rem;
		}

		dl:not(.compact) dt {
			padding-top: 0.45rem;
			font-size: 0.75rem;
		}

		dl:not(.compact) dt:first-of-type {
			padding-top: 0;
		}
	}

	/* 手指点的地方要大一些。 */
	@media (pointer: coarse) {
		.save {
			padding: 0.25rem 0.3rem;
		}
	}

	.save:hover,
	.save.on {
		color: var(--accent, #3b97ea);
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
