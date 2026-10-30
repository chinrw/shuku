<script lang="ts">
	import { untrack } from 'svelte';
	import { replaceState } from '$app/navigation';
	import { fittedWidth, pageUrl, widthTier } from '#lib/image.ts';
	import { ProgressReporter } from '#lib/progress.ts';
	import { readerSettings } from '#lib/settings.svelte.ts';

	// 滚动模式下页面不会比这个宽，宽屏上两侧留空。
	const SCROLL_MAX_WIDTH = 960;

	let { data } = $props();
	const settings = readerSettings();
	const book = untrack(() => data.archive);
	const pages = untrack(() => data.pages);

	let current = $state(untrack(() => data.start));
	let toolbar = $state(false);
	let progressError = $state<string | null>(null);
	// 关闭了 SSR，这里一定在浏览器里运行。先取一次真实值，免得第一次渲染按宽度 0 请求图片。
	let innerWidth = $state(window.innerWidth);
	let innerHeight = $state(window.innerHeight);
	let devicePixelRatio = $state(window.devicePixelRatio);
	let scroller = $state<HTMLElement>();

	const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
	const reporter = new ProgressReporter(book.id, (error) => {
		progressError = errorText(error);
	});

	// 只在翻页时上报。打开时不报，免得用旧页码和新时间戳盖掉别的设备更新的进度。
	function record(page: number) {
		current = page;
		progressError = null;
		reporter.turn(page);
		replaceState(`?page=${page}`, {});
	}

	function go(page: number) {
		if (page >= 0 && page < pages.length && page !== current) record(page);
	}

	function src(index: number, cssWidth: number) {
		return pageUrl(pages[index], widthTier(cssWidth, devicePixelRatio));
	}

	function pagedSrc(index: number) {
		return src(index, fittedWidth(pages[index], { width: innerWidth, height: innerHeight }));
	}

	// 预取前后几页，翻页时直接从 HTTP 缓存取图。
	$effect(() => {
		if (settings.mode !== 'paged') return;
		for (const index of [current + 1, current + 2, current - 1]) {
			if (index >= 0 && index < pages.length) new Image().src = pagedSrc(index);
		}
	});

	$effect(() => {
		const onHide = () => {
			if (document.visibilityState === 'hidden') {
				reporter.flush(true).catch((error) => (progressError = errorText(error)));
			}
		};
		document.addEventListener('visibilitychange', onHide);
		return () => {
			document.removeEventListener('visibilitychange', onHide);
			reporter.flush(true).catch((error) => console.error('保存阅读进度失败', error));
		};
	});

	function tap(event: MouseEvent) {
		const x = event.clientX / innerWidth;
		if (x > 1 / 3 && x < 2 / 3) {
			toolbar = !toolbar;
			return;
		}
		const forward = x >= 2 / 3 === (settings.direction === 'ltr');
		go(current + (forward ? 1 : -1));
	}

	function key(event: KeyboardEvent) {
		if (settings.mode !== 'paged') return;
		const rtl = settings.direction === 'rtl';
		switch (event.key) {
			case 'ArrowLeft':
				go(current + (rtl ? 1 : -1));
				break;
			case 'ArrowRight':
				go(current + (rtl ? -1 : 1));
				break;
			case ' ':
			case 'PageDown':
				go(current + 1);
				break;
			case 'PageUp':
				go(current - 1);
				break;
			default:
				return;
		}
		event.preventDefault();
	}

	// 页面中线所在的那一页算当前页。
	function trackScroll(node: HTMLElement) {
		const observer = new IntersectionObserver(
			(entries) => {
				for (const entry of entries) {
					const index = Number((entry.target as HTMLElement).dataset.index);
					if (entry.isIntersecting && index !== current) record(index);
				}
			},
			{ root: node, rootMargin: '-50% 0px -50% 0px' }
		);
		for (const child of node.children) observer.observe(child);
		// 每页都按宽高比占好位置，图片还没加载也能直接滚到目标页。
		node.children[untrack(() => current)]?.scrollIntoView();
		return () => observer.disconnect();
	}

	function jump(page: number) {
		if (settings.mode === 'scroll') scroller?.children[page]?.scrollIntoView();
		else go(page);
	}

	function setMode(mode: typeof settings.mode) {
		settings.mode = mode;
		settings.save();
	}

	function toggleDirection() {
		settings.direction = settings.direction === 'rtl' ? 'ltr' : 'rtl';
		settings.save();
	}
</script>

<svelte:head>
	<title>{book.title} · Shuku</title>
</svelte:head>

<svelte:window bind:innerWidth bind:innerHeight bind:devicePixelRatio onkeydown={key} />

<div class="reader">
	{#if pages.length === 0}
		<p class="empty">这本书没有可显示的页面。</p>
	{:else if settings.mode === 'paged'}
		<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
		<div class="paged" onclick={tap}>
			<img
				class="page"
				src={pagedSrc(current)}
				width={pages[current].width}
				height={pages[current].height}
				alt="第 {current + 1} 页"
			/>
		</div>
	{:else}
		<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
		<div class="scroll" bind:this={scroller} onclick={() => (toolbar = !toolbar)} {@attach trackScroll}>
			{#each pages as page, index (index)}
				<img
					data-index={index}
					src={src(index, Math.min(innerWidth, SCROLL_MAX_WIDTH))}
					width={page.width}
					height={page.height}
					loading={Math.abs(index - current) <= 1 ? 'eager' : 'lazy'}
					alt="第 {index + 1} 页"
				/>
			{/each}
		</div>
	{/if}

	{#if toolbar}
		<header class="bar">
			<a href="/a/{book.id}">‹</a>
			<span class="title">{book.title}</span>
			<span>{current + 1} / {pages.length}</span>
		</header>
		<footer class="bar">
			<input
				type="range"
				min="0"
				max={Math.max(pages.length - 1, 0)}
				value={current}
				dir={settings.mode === 'paged' && settings.direction === 'rtl' ? 'rtl' : 'ltr'}
				onchange={(event) => jump(Number(event.currentTarget.value))}
			/>
			<button onclick={() => setMode(settings.mode === 'paged' ? 'scroll' : 'paged')}>
				{settings.mode === 'paged' ? '改为滚动' : '改为翻页'}
			</button>
			{#if settings.mode === 'paged'}
				<button onclick={toggleDirection}>
					{settings.direction === 'rtl' ? '从右往左' : '从左往右'}
				</button>
			{/if}
		</footer>
	{/if}

	{#if progressError}
		<p class="toast">进度没有保存：{progressError}</p>
	{/if}
</div>

<style>
	.reader {
		position: fixed;
		inset: 0;
		background: #000;
		color: #eee;
		user-select: none;
		-webkit-user-select: none;
	}

	.paged {
		position: absolute;
		inset: 0;
	}

	.page {
		position: absolute;
		inset: 0;
		margin: auto;
		max-width: 100%;
		max-height: 100%;
		width: auto;
		height: auto;
	}

	.scroll {
		height: 100%;
		overflow-y: auto;
		overscroll-behavior: contain;
	}

	.scroll img {
		display: block;
		width: 100%;
		max-width: 960px;
		height: auto;
		margin: 0 auto;
	}

	.bar {
		position: absolute;
		left: 0;
		right: 0;
		display: flex;
		align-items: center;
		gap: 0.75rem;
		padding: 0.6rem 1rem;
		background: rgb(0 0 0 / 0.8);
	}

	header.bar {
		top: 0;
		padding-top: max(0.6rem, env(safe-area-inset-top));
	}

	footer.bar {
		bottom: 0;
		padding-bottom: max(0.6rem, env(safe-area-inset-bottom));
	}

	.bar a {
		font-size: 1.4rem;
		text-decoration: none;
	}

	.title {
		flex: 1;
		overflow: hidden;
		white-space: nowrap;
		text-overflow: ellipsis;
	}

	input[type='range'] {
		flex: 1;
	}

	button {
		padding: 0.35rem 0.75rem;
		border: 1px solid #555;
		border-radius: 0.4rem;
		background: transparent;
	}

	.toast {
		position: absolute;
		left: 50%;
		bottom: 4.5rem;
		transform: translateX(-50%);
		margin: 0;
		padding: 0.4rem 0.8rem;
		border-radius: 0.4rem;
		background: #7f1d1d;
		font-size: 0.85rem;
	}

	.empty {
		padding: 2rem;
		text-align: center;
	}
</style>
