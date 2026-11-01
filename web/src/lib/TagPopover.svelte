<script lang="ts">
	import type { ArchiveItem } from './api';
	import { fade, slide } from './motion';
	import TagTable from './TagTable.svelte';
	import { tagRows } from './tags';

	// 宽屏是贴着标签行的浮层；手机上屏幕太窄，改成底部弹出的面板。
	let {
		item,
		anchor,
		sheet,
		onclose,
		onpointerenter,
		onpointerleave
	}: {
		item: ArchiveItem;
		anchor: DOMRect;
		sheet: boolean;
		onclose: () => void;
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

<!-- 底部面板滑上来、遮罩淡入；宽屏的浮层直接出现。关上时直接消失，书库页这时已经把 item 清空了。
	遮罩不放进 {#if} 里，否则它不是随这个组件一起出现的，进场动画不会播放。 -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="scrim" class:hidden={!sheet} onclick={onclose} in:fade></div>
<div
	class="popover"
	class:sheet
	style={sheet ? undefined : position}
	in:slide={{ from: sheet ? 'bottom' : null }}
	role="dialog"
	tabindex="-1"
	aria-label="{item.title} 的标签"
	{onpointerenter}
	{onpointerleave}
>
	{#if sheet}
		<div class="head">
			<a class="title" href="/a/{item.id}">{item.title}</a>
		</div>
	{/if}
	<TagTable rows={tagRows(item)} saveable />
</div>

<style>
	.popover {
		position: fixed;
		z-index: 20;
		overflow-y: auto;
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

	.scrim.hidden {
		display: none;
	}

	.head {
		margin-bottom: 0.75rem;
	}

	.title {
		font-weight: 600;
		line-height: 1.4;
		text-decoration: none;
	}
</style>
