<script lang="ts">
	import { untrack } from 'svelte';
	import { afterNavigate, replaceState } from '$app/navigation';
	import CategoryPicker from '#lib/CategoryPicker.svelte';
	import Icon from '#lib/Icon.svelte';
	import { fittedWidth, pageUrl, widthTier } from '#lib/image.ts';
	import { ProgressReporter } from '#lib/progress.ts';
	import { settings } from '#lib/settings.svelte.ts';
	import { buildSpreads } from '#lib/spreads.ts';
	import TagTable from '#lib/TagTable.svelte';
	import { tagRows } from '#lib/tags.ts';

	// 滚动模式下页面不会比这个宽，宽屏上两侧留空。
	const SCROLL_MAX_WIDTH = 960;

	let { data } = $props();
	const prefs = settings();
	const book = untrack(() => data.archive);
	const pages = untrack(() => data.pages);
	let categoryList = $state(untrack(() => data.categories));
	let members = $state(untrack(() => data.archive.categories));

	let current = $state(untrack(() => data.start));
	let toolbar = $state(false);
	let panel = $state(false);
	let progressError = $state<string | null>(null);
	// 关闭了 SSR，这里一定在浏览器里运行。先取一次真实值，免得第一次渲染按宽度 0 请求图片。
	let innerWidth = $state(window.innerWidth);
	let innerHeight = $state(window.innerHeight);
	let devicePixelRatio = $state(window.devicePixelRatio);
	let scroller = $state<HTMLElement>();

	// 双页只在横屏时生效，竖屏手机上一屏放两页太小。
	let spreads = $derived(
		prefs.spread && innerWidth > innerHeight ? buildSpreads(pages) : pages.map((_, index) => [index])
	);
	let spreadIndex = $derived(Math.max(spreads.findIndex((spread) => spread.includes(current)), 0));
	let visible = $derived(spreads[spreadIndex] ?? []);

	let fromDetail = false;
	afterNavigate(({ from }) => {
		fromDetail = from?.route.id === '/a/[id]';
	});

	function back(event: MouseEvent) {
		if (!fromDetail) return;
		event.preventDefault();
		history.back();
	}

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

	function step(delta: number) {
		const target = spreads[spreadIndex + delta];
		if (target) go(target[0]);
	}

	function src(index: number, cssWidth: number) {
		return pageUrl(pages[index], widthTier(cssWidth, devicePixelRatio));
	}

	// 双页时每页只占半个屏幕宽。
	function pagedSrc(index: number, count: number) {
		const viewport = { width: innerWidth / count, height: innerHeight };
		return src(index, fittedWidth(pages[index], viewport));
	}

	// 预取前后的几屏，翻页时直接从 HTTP 缓存取图。
	$effect(() => {
		if (prefs.mode !== 'paged') return;
		for (const offset of [1, 2, -1]) {
			const spread = spreads[spreadIndex + offset];
			for (const index of spread ?? []) new Image().src = pagedSrc(index, spread.length);
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
		const forward = x >= 2 / 3 === (prefs.direction === 'ltr');
		step(forward ? 1 : -1);
	}

	function key(event: KeyboardEvent) {
		if (event.key === 'Escape' && panel) {
			panel = false;
			return;
		}
		if (prefs.mode !== 'paged' || panel) return;
		const rtl = prefs.direction === 'rtl';
		switch (event.key) {
			case 'ArrowLeft':
				step(rtl ? 1 : -1);
				break;
			case 'ArrowRight':
				step(rtl ? -1 : 1);
				break;
			case ' ':
			case 'PageDown':
				step(1);
				break;
			case 'PageUp':
				step(-1);
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
		if (prefs.mode === 'scroll') scroller?.children[page]?.scrollIntoView();
		else go(spreads.find((spread) => spread.includes(page))?.[0] ?? page);
	}

	function update(change: () => void) {
		change();
		prefs.save();
	}
</script>

<svelte:head>
	<title>{book.title} · Shuku</title>
</svelte:head>

<svelte:window bind:innerWidth bind:innerHeight bind:devicePixelRatio onkeydown={key} />

<div class="reader">
	{#if pages.length === 0}
		<p class="empty">这本书没有可显示的页面。</p>
	{:else if prefs.mode === 'paged'}
		<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
		<div class="paged" class:rtl={prefs.direction === 'rtl'} onclick={tap}>
			{#each visible as index (index)}
				<img
					class="page"
					class:half={visible.length === 2}
					src={pagedSrc(index, visible.length)}
					width={pages[index].width}
					height={pages[index].height}
					alt="第 {index + 1} 页"
				/>
			{/each}
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

	<!-- 平时不显示任何控件；点屏幕中间出现一条底栏，设置和标签都收在菜单里。 -->
	{#if toolbar}
		<footer class="bar">
			<a class="bar-button" href="/a/{book.id}" onclick={back} aria-label="返回"><Icon name="back" /></a>
			<input
				type="range"
				min="0"
				max={Math.max(pages.length - 1, 0)}
				value={current}
				dir={prefs.mode === 'paged' && prefs.direction === 'rtl' ? 'rtl' : 'ltr'}
				onchange={(event) => jump(Number(event.currentTarget.value))}
				aria-label="跳到页"
			/>
			<span class="counter">
				{visible.length === 2 ? `${visible[0] + 1}-${visible[1] + 1}` : current + 1} / {pages.length}
			</span>
			<button class="bar-button" onclick={() => (panel = true)} aria-label="菜单">
				<Icon name="more" />
			</button>
		</footer>
	{/if}

	{#if panel}
		<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
		<div class="scrim" onclick={() => (panel = false)}></div>
		<aside class="panel" aria-label="设置和标签">
			<div class="panel-head">
				<h2>{book.title}</h2>
				<button class="bar-button" onclick={() => (panel = false)} aria-label="关闭">
					<Icon name="close" />
				</button>
			</div>
			<div class="settings">
				<span>阅读方式</span>
				<div class="segmented">
					<button class:on={prefs.mode === 'paged'} onclick={() => update(() => (prefs.mode = 'paged'))}
						>翻页</button
					>
					<button class:on={prefs.mode === 'scroll'} onclick={() => update(() => (prefs.mode = 'scroll'))}
						>滚动</button
					>
				</div>
				{#if prefs.mode === 'paged'}
					<span>方向</span>
					<div class="segmented">
						<button
							class:on={prefs.direction === 'rtl'}
							onclick={() => update(() => (prefs.direction = 'rtl'))}>右→左</button
						>
						<button
							class:on={prefs.direction === 'ltr'}
							onclick={() => update(() => (prefs.direction = 'ltr'))}>左→右</button
						>
					</div>
					<span>横屏双页</span>
					<div class="segmented">
						<button class:on={prefs.spread} onclick={() => update(() => (prefs.spread = true))}>开</button>
						<button class:on={!prefs.spread} onclick={() => update(() => (prefs.spread = false))}
							>关</button
						>
					</div>
				{/if}
			</div>
			<CategoryPicker archive={book.id} bind:categories={categoryList} bind:members />
			<TagTable rows={tagRows(book)} />
			{#if book.summary}<p class="summary">{book.summary}</p>{/if}
		</aside>
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
		display: flex;
		align-items: center;
		justify-content: center;
	}

	/* 从右往左读时，第一页放在右边。 */
	.paged.rtl {
		flex-direction: row-reverse;
	}

	.page {
		max-width: 100%;
		max-height: 100%;
		width: auto;
		height: auto;
		object-fit: contain;
	}

	.page.half {
		max-width: 50%;
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
		bottom: 0;
		display: flex;
		align-items: center;
		gap: 0.5rem;
		padding: 0.4rem 0.6rem;
		padding-bottom: max(0.4rem, env(safe-area-inset-bottom));
		background: rgb(20 20 22 / 0.88);
		backdrop-filter: blur(8px);
		-webkit-backdrop-filter: blur(8px);
	}

	.bar-button {
		display: inline-flex;
		flex: none;
		align-items: center;
		justify-content: center;
		width: 2.25rem;
		height: 2.25rem;
		border: none;
		background: transparent;
		color: inherit;
		cursor: pointer;
	}

	.counter {
		flex: none;
		font-size: 0.85rem;
		font-variant-numeric: tabular-nums;
		color: #bbb;
	}

	input[type='range'] {
		flex: 1;
		min-width: 0;
		accent-color: #8a8d93;
	}

	.settings {
		display: grid;
		grid-template-columns: max-content 1fr;
		align-items: center;
		gap: 0.6rem 1rem;
		margin-bottom: 1.25rem;
		font-size: 0.9rem;
		color: #a6a9ae;
	}

	.segmented {
		display: flex;
		width: fit-content;
		border: 1px solid #50545c;
		border-radius: 6px;
		overflow: hidden;
	}

	.segmented button {
		padding: 0.3rem 0.9rem;
		border: none;
		background: transparent;
		color: #ccc;
		font-size: 0.85rem;
		cursor: pointer;
	}

	.segmented button.on {
		background: #4f535b;
		color: #fff;
	}

	.scrim {
		position: absolute;
		inset: 0;
		background: rgb(0 0 0 / 0.45);
	}

	/* 宽屏是右侧抽屉，手机上是底部弹出的面板。 */
	.panel {
		--fg: #dddddd;
		--muted: #a6a9ae;
		--chip: #4f535b;
		--chip-border: #6d7179;
		--link: #3b97ea;
		position: absolute;
		top: 0;
		right: 0;
		bottom: 0;
		width: min(26rem, 100%);
		overflow-y: auto;
		padding: 1rem;
		padding-top: max(1rem, env(safe-area-inset-top));
		background: #34353b;
		user-select: text;
		-webkit-user-select: text;
	}

	@media (max-width: 599px) {
		.panel {
			top: auto;
			width: 100%;
			max-height: 75%;
			border-radius: 12px 12px 0 0;
			padding-bottom: max(1rem, env(safe-area-inset-bottom));
		}
	}

	.panel-head {
		display: flex;
		align-items: flex-start;
		gap: 0.5rem;
		margin-bottom: 0.75rem;
	}

	.panel h2 {
		flex: 1;
		margin: 0.35rem 0 0;
		font-size: 1rem;
		line-height: 1.4;
	}

	.summary {
		margin: 1rem 0 0;
		line-height: 1.7;
		white-space: pre-wrap;
		color: #ccc;
	}

	.toast {
		position: absolute;
		left: 50%;
		bottom: 5.5rem;
		transform: translateX(-50%);
		margin: 0;
		padding: 0.4rem 0.8rem;
		border-radius: 6px;
		background: #7f1d1d;
		font-size: 0.85rem;
	}

	.empty {
		padding: 2rem;
		text-align: center;
	}
</style>
