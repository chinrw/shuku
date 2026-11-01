<script lang="ts">
	import { afterNavigate } from '$app/navigation';
	import Icon from '#lib/Icon.svelte';
	import { nextVolume } from '#lib/series.ts';

	// 系列页：一部作品的各卷按卷号排列，“继续阅读”打开接下来该读的那一卷。
	let { data } = $props();
	let volumes = $derived(data.series.items);
	let resume = $derived(nextVolume(volumes));
	let finished = $derived(
		volumes.filter((item) => item.progress && item.progress.page + 1 >= item.page_count).length
	);

	// 标题去掉前面的系列名：“作品 第1卷”显示成“第1卷”，按话分的“作品 032”显示成“032”。
	function label(item: (typeof volumes)[number]) {
		const prefix = `${data.series.name} `;
		return item.title.startsWith(prefix) ? item.title.slice(prefix.length) : item.title;
	}

	// 从书库点进来时用浏览器返回，书库的滚动位置才会恢复。
	let fromLibrary = false;
	afterNavigate(({ from }) => (fromLibrary = from?.route.id === '/'));
	function back(event: MouseEvent) {
		if (!fromLibrary) return;
		event.preventDefault();
		history.back();
	}
</script>

<svelte:head>
	<title>{data.series.name} · Shuku</title>
</svelte:head>

<nav>
	<a class="icon-button" href="/" onclick={back} aria-label="返回书库"><Icon name="back" /></a>
</nav>
<main>
	<header>
		<h1>{data.series.name}</h1>
		<p class="meta">共 {volumes.length} 卷{#if finished}，读完 {finished} 卷{/if}</p>
		{#if resume}
			<a class="primary" href="/a/{resume.id}/read">
				{resume.progress ? '继续阅读' : '开始阅读'} · {label(resume)}
			</a>
		{/if}
	</header>
	<ol class="volumes">
		{#each volumes as item (item.id)}
			{@const page = item.progress?.page ?? -1}
			<li>
				<a class="volume" href="/a/{item.id}/read" title={item.title}>
					<span class="cover" class:current={item.id === resume?.id}>
						<img src={item.cover_url} alt="" loading="lazy" />
						{#if item.progress && item.page_count > 0}
							<span class="progress" style:width="{((page + 1) / item.page_count) * 100}%"></span>
						{/if}
					</span>
					<span class="label">{label(item)}</span>
					<span class="pages">
						{#if item.progress}{page + 1}/{item.page_count}{:else}{item.page_count} 页{/if}
					</span>
				</a>
				<a class="info" href="/a/{item.id}" aria-label="{label(item)}的详情">详情</a>
			</li>
		{/each}
	</ol>
</main>

<style>
	nav {
		padding: 0.6rem 1rem 0;
		padding-top: max(0.6rem, env(safe-area-inset-top));
	}

	main {
		max-width: 64rem;
		margin: 0 auto;
		padding: 0.5rem 1rem 2rem;
	}

	h1 {
		margin: 0 0 0.3rem;
		font-size: 1.3rem;
		line-height: 1.4;
		overflow-wrap: anywhere;
	}

	.meta {
		margin: 0 0 0.9rem;
		color: var(--muted);
		font-size: 0.85rem;
	}

	.primary {
		display: inline-block;
		padding: 0.5rem 0.9rem;
		border-radius: 6px;
		background: var(--accent);
		color: var(--on-accent);
		text-decoration: none;
		font-size: 0.9rem;
	}

	.volumes {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
		gap: 1rem 0.9rem;
		margin: 1.4rem 0 0;
		padding: 0;
		list-style: none;
	}

	li {
		display: flex;
		flex-direction: column;
		gap: 0.2rem;
	}

	.volume {
		display: flex;
		flex-direction: column;
		gap: 0.3rem;
		text-decoration: none;
		font-size: 0.85rem;
	}

	.cover {
		position: relative;
		display: block;
		overflow: hidden;
		border-radius: 6px;
		background: var(--chip);
		outline: 2px solid transparent;
		outline-offset: 2px;
		transition: outline-color 0.15s;
	}

	/* 接下来该读的那一卷描一圈强调色。 */
	.cover.current {
		outline-color: var(--accent);
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

	.label {
		font-weight: 500;
	}

	.volume:hover .label {
		color: var(--link);
	}

	.pages,
	.info {
		color: var(--muted);
		font-size: 0.75rem;
	}

	.info {
		align-self: flex-start;
	}

	.info:hover {
		color: var(--link);
	}
</style>
