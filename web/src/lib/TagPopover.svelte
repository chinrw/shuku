<script lang="ts">
	import type { ArchiveItem, SeriesRef } from './api';
	import Icon from './Icon.svelte';
	import { fade, slide } from './motion';
	import TagTable from './TagTable.svelte';
	import { tagRows } from './tags';

	// 宽屏是贴着标签行的浮层；窄屏上屏幕太窄，改成从底部滑出的标签面板。
	let {
		item,
		series = null,
		anchor,
		sheet,
		onclose,
		ondetail,
		onpointerenter,
		onpointerleave
	}: {
		item: ArchiveItem;
		series?: SeriesRef | null;
		anchor: DOMRect;
		sheet: boolean;
		onclose: () => void;
		ondetail?: () => void;
		onpointerenter: (event: PointerEvent) => void;
		onpointerleave: (event: PointerEvent) => void;
	} = $props();

	const GAP = 6;
	const MARGIN = 8;
	const WIDTH = 420;

	// 下方放得下就放在标签行下面，否则放在上面，最大高度取那一侧剩余的空间。
	let position = $derived.by(() => {
		const width = Math.min(WIDTH, window.innerWidth - MARGIN * 2);
		const left = Math.min(Math.max(MARGIN, anchor.left), window.innerWidth - width - MARGIN);
		const below = window.innerHeight - anchor.bottom - GAP - MARGIN;
		const above = anchor.top - GAP - MARGIN;
		return below >= Math.min(above, 320)
			? `left:${left}px;width:${width}px;top:${anchor.bottom + GAP}px;max-height:${below}px`
			: `left:${left}px;width:${width}px;bottom:${window.innerHeight - anchor.top + GAP}px;max-height:${above}px`;
	});
</script>

<!-- 窄屏上是底部滑入滑出的面板和遮罩；宽屏上 motion.ts 对 from: null 使用 0ms，浮层立即出现和消失。 -->
{#if sheet}
	<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
	<div class="scrim" onclick={onclose} in:fade out:fade></div>
{/if}
<div
	class="popover"
	class:sheet
	style={sheet ? undefined : position}
	in:slide={{ from: sheet ? 'bottom' : null }}
	out:slide={{ from: sheet ? 'bottom' : null }}
	role="dialog"
	tabindex="-1"
	aria-label="{series ? series.name : item.title} 的标签"
	{onpointerenter}
	{onpointerleave}
>
	{#if sheet}
		<div class="head">
			{#if series}
				<a class="title" href="/s/{series.id}">{series.name}</a>
			{:else if ondetail}
				<button class="title" onclick={ondetail}>{item.title}</button>
			{:else}
				<a class="title" href="/a/{item.id}">{item.title}</a>
			{/if}
			<button class="close" onclick={onclose} aria-label="关闭标签">
				<Icon name="close" size={18} />
			</button>
		</div>
	{/if}
	<TagTable rows={tagRows(item)} saveable />
</div>

<style>
	.popover {
		position: fixed;
		z-index: 20;
		overflow-y: auto;
		overscroll-behavior: contain;
		padding: 0.75rem;
		border: 1px solid var(--border);
		border-radius: 10px;
		background: var(--raised);
		box-shadow: 0 12px 32px rgb(0 0 0 / 0.35);
	}

	.popover.sheet {
		left: 0;
		right: 0;
		bottom: 0;
		max-height: 70%;
		border-radius: 14px 14px 0 0;
		padding: 1rem 1rem max(1rem, env(safe-area-inset-bottom));
	}

	.scrim {
		position: fixed;
		inset: 0;
		z-index: 19;
		background: rgb(0 0 0 / 0.45);
	}

	.head {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: 0.75rem;
		margin-bottom: 0.75rem;
	}

	.title {
		min-width: 0;
		padding: 0;
		border: none;
		background: none;
		color: inherit;
		font: inherit;
		font-weight: 600;
		line-height: 1.4;
		text-align: left;
		text-decoration: none;
		overflow-wrap: anywhere;
		cursor: pointer;
	}

	.title:hover {
		color: var(--link);
	}

	.close {
		flex: none;
		display: inline-flex;
		align-items: center;
		justify-content: center;
		margin: -0.25rem -0.25rem 0 0;
		padding: 0.35rem;
		border: none;
		background: transparent;
		color: var(--muted);
		cursor: pointer;
	}

	.close:hover {
		color: var(--fg);
	}
</style>
