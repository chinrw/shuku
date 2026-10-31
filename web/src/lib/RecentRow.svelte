<script lang="ts">
	import type { ArchiveItem } from './api';
	import { ago } from './time';

	// 书库首页顶部的一排最近读过的本子，点封面直接继续读。链接不带页码：这里的进度来自
	// 搜索快照，可能落后约 10 秒，阅读器自己读详情接口的最新进度。
	let { items }: { items: ArchiveItem[] } = $props();
	const now = Date.now();
</script>

<section aria-labelledby="recent-heading">
	<h2 id="recent-heading">最近阅读</h2>
	<div class="strip">
		{#each items as item (item.id)}
			{@const page = item.progress?.page ?? 0}
			<a class="book" href="/a/{item.id}/read" title={item.title}>
				<span class="cover">
					<img src={item.cover_url} alt="" loading="lazy" />
					{#if item.page_count > 0}
						<span class="progress" style:width="{((page + 1) / item.page_count) * 100}%"></span>
					{/if}
				</span>
				<span class="title">{item.title}</span>
				<span class="meta">
					{page + 1}/{item.page_count}
					{#if item.progress}· {ago(item.progress.client_time, now)}{/if}
				</span>
			</a>
		{/each}
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

	.book:hover .title {
		color: var(--link);
	}
</style>
