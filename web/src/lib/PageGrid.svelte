<script lang="ts">
	import type { PageInfo } from '#lib/api.ts';

	// 详情页和阅读器的详情面板共用。阅读器里换页不能走路由导航（同一路由不会重新挂载），
	// 所以给了 onpick 时拦下点击，由阅读器自己翻页。
	let {
		archive,
		pages,
		current = null,
		onpick
	}: {
		archive: number;
		pages: PageInfo[];
		current?: number | null;
		onpick?: (page: number) => void;
	} = $props();

	function pick(event: MouseEvent, page: number) {
		// 带修饰键的点击（新标签页打开等）照常走链接。
		const plain = event.button === 0 && !(event.ctrlKey || event.metaKey || event.shiftKey || event.altKey);
		if (!onpick || !plain) return;
		event.preventDefault();
		onpick(page);
	}
</script>

<section class="pages" aria-label="页面">
	{#each pages as page, index (index)}
		<a
			href="/a/{archive}/read?page={index}"
			class:current={index === current}
			aria-current={index === current ? 'page' : undefined}
			onclick={(event) => pick(event, index)}
		>
			<img
				src={page.thumbnail_url}
				width={page.width}
				height={page.height}
				loading="lazy"
				alt="第 {index + 1} 页"
			/>
			<span>{index + 1}</span>
		</a>
	{/each}
</section>

<style>
	/* 缩略图按原图宽高比占位，懒加载时网格不会跳动。 */
	.pages {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(96px, 1fr));
		gap: 0.75rem 0.6rem;
	}

	@media (min-width: 720px) {
		.pages {
			grid-template-columns: repeat(auto-fill, minmax(132px, 1fr));
		}
	}

	.pages a {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 0.25rem;
		color: var(--muted);
		font-size: 0.8rem;
		text-decoration: none;
	}

	/* 长条漫画的页很长，只显示顶部一截。 */
	.pages img {
		width: 100%;
		height: auto;
		max-height: 18rem;
		object-fit: cover;
		object-position: top;
		border: 2px solid transparent;
		border-radius: 4px;
		background: var(--chip);
	}

	.pages a.current img {
		border-color: var(--accent);
	}

	.pages a.current {
		color: var(--accent);
	}
</style>
