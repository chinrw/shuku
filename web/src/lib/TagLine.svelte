<script lang="ts">
	import type { TagGroup } from './api';
	import { namespaceClass, orderGroups } from './tags';

	// LRR 卡片底部那一行：只显示标签值，按命名空间着色。
	let { groups }: { groups: TagGroup[] } = $props();
	let tags = $derived(
		orderGroups(groups).flatMap((group) =>
			group.values.map((value) => ({ value, color: namespaceClass(group.namespace) }))
		)
	);
</script>

<span class="tagline">
	{#each tags as tag, index (index)}<span class={tag.color}>{tag.value}</span
		>{#if index < tags.length - 1}<span class="comma">{', '}</span>{/if}{/each}
</span>

<style>
	.tagline {
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
		font-size: 0.78rem;
		line-height: 1.35;
		color: var(--muted);
	}

	.comma {
		color: var(--faint);
	}
</style>
