<script lang="ts">
	import type { ArchiveItem } from './api';
	import FinishedBadge from './FinishedBadge.svelte';
	import Icon from './Icon.svelte';
	import { ago } from './time';

	// 书库首页顶部的一排最近读过的本子，点封面直接继续读。链接不带页码：这里的进度来自
	// 搜索快照，可能落后约 10 秒，阅读器自己读详情接口的最新进度。
	// 和书库的卡片一样：点封面接着读，点标题打开详情抽屉（ondetail）。
	let { items, ondetail }: { items: ArchiveItem[]; ondetail: (item: ArchiveItem) => void } =
		$props();
	const now = Date.now();

	// 和 LRR 一样是横向轮播：触屏直接滑，电脑上两侧有按钮，每次翻过大半屏。
	let strip = $state<HTMLElement>();
	let atStart = $state(true);
	let atEnd = $state(true);

	function measure() {
		if (!strip) return;
		atStart = strip.scrollLeft <= 1;
		atEnd = strip.scrollLeft + strip.clientWidth >= strip.scrollWidth - 1;
	}

	function page(direction: 1 | -1) {
		strip?.scrollBy({ left: direction * strip.clientWidth * 0.8, behavior: 'smooth' });
	}

	// 宽度和本数变了，能不能往两边翻也跟着变。
	$effect(() => {
		void items.length;
		if (!strip) return;
		const observer = new ResizeObserver(measure);
		observer.observe(strip);
		return () => observer.disconnect();
	});
</script>

<section aria-labelledby="recent-heading">
	<h2 id="recent-heading">最近阅读</h2>
	<div class="carousel">
	<button class="nav prev" onclick={() => page(-1)} disabled={atStart} aria-label="往前">
		<Icon name="chevron-left" />
	</button>
	<div class="strip" bind:this={strip} onscroll={measure}>
		{#each items as item (item.id)}
			{@const page = item.progress?.page ?? 0}
			<div class="book">
				<a class="cover" href="/a/{item.id}/read" aria-label="接着读 {item.title}">
					<img src={item.cover_url} alt="" loading="lazy" />
					{#if item.progress?.finished}<FinishedBadge />{/if}
					{#if item.page_count > 0}
						<span class="progress" style:width="{((page + 1) / item.page_count) * 100}%"></span>
					{/if}
				</a>
				<button class="title" title={item.title} onclick={() => ondetail(item)}>{item.title}</button>
				<span class="meta">
					{page + 1}/{item.page_count}
					{#if item.progress}· {ago(item.progress.client_time, now)}{/if}
				</span>
			</div>
		{/each}
	</div>
	<button class="nav next" onclick={() => page(1)} disabled={atEnd} aria-label="往后">
		<Icon name="chevron-right" />
	</button>
	</div>
</section>

<style>
	section {
		padding: 0.4rem 1rem 0.2rem;
	}

	h2 {
		margin: 0 0 0.5rem;
		font-size: 0.95rem;
	}

	.carousel {
		position: relative;
	}

	/* 按钮压在两端封面上，只给有鼠标的设备；到头的一侧隐藏。 */
	.nav {
		position: absolute;
		top: 0;
		z-index: 1;
		display: none;
		align-items: center;
		justify-content: center;
		width: 2.25rem;
		height: calc(7.5rem * 7 / 5);
		border: none;
		background: color-mix(in srgb, var(--bg) 75%, transparent);
		color: var(--fg);
		cursor: pointer;
	}

	.nav.prev {
		left: 0;
		border-radius: 6px 0 0 6px;
	}

	.nav.next {
		right: 0;
		border-radius: 0 6px 6px 0;
	}

	@media (hover: hover) {
		.nav {
			display: flex;
		}

		.nav:disabled {
			display: none;
		}
	}

	.strip {
		display: grid;
		grid-auto-flow: column;
		grid-auto-columns: 7.5rem;
		gap: 0.75rem;
		overflow-x: auto;
		padding-bottom: 0.5rem;
		scroll-snap-type: x proximity;
	}

	.book {
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
		scroll-snap-align: start;
		text-decoration: none;
		font-size: 0.78rem;
		line-height: 1.3;
	}

	.cover {
		position: relative;
		display: block;
		overflow: hidden;
		border-radius: 6px;
		background: var(--chip);
	}

	.cover img {
		display: block;
		width: 100%;
		aspect-ratio: 5 / 7;
		object-fit: cover;
		object-position: center top;
	}

	.progress {
		position: absolute;
		left: 0;
		bottom: 0;
		height: 4px;
		background: var(--accent);
	}

	.title {
		padding: 0;
		border: none;
		background: none;
		color: inherit;
		font: inherit;
		text-align: left;
		cursor: pointer;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}

	.meta {
		color: var(--muted);
		font-size: 0.72rem;
	}

	.title:hover {
		color: var(--link);
	}
</style>
