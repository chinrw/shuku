<script lang="ts">
	import { untrack } from 'svelte';
	import { afterNavigate, replaceState } from '$app/navigation';
	import { downloadUrl } from '#lib/api.ts';
	import BookmarkButton from '#lib/BookmarkButton.svelte';
	import CategoryPicker from '#lib/CategoryPicker.svelte';
	import Icon from '#lib/Icon.svelte';
	import { fittedWidth, pageUrl, widthTier } from '#lib/image.ts';
	import PageGrid from '#lib/PageGrid.svelte';
	import { ProgressReporter } from '#lib/progress.ts';
	import { settings, WEBTOON_WIDTH } from '#lib/settings.svelte.ts';
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
	const members = untrack(() => data.archive.categories);

	let current = $state(untrack(() => data.start));
	// 平时顶部有工具栏，和 LRR 一样；全屏时收起，点屏幕中间临时叫出底栏。
	let immersive = $state(false);
	let overlay = $state(false);
	// “详情”面板就是这本书的详情页：标签、分类、下载和全部页面的缩略图。
	let panel = $state<'detail' | 'settings' | null>(untrack(() => data.detail) ? 'detail' : null);
	let progressError = $state<string | null>(null);
	// 关闭了 SSR，这里一定在浏览器里运行。先取一次真实值，免得第一次渲染按宽度 0 请求图片。
	let innerWidth = $state(window.innerWidth);
	let innerHeight = $state(window.innerHeight);
	let devicePixelRatio = $state(window.devicePixelRatio);
	// 图片区域的大小，不含工具栏。挂载前先按整个窗口估计。
	let stageWidth = $state(window.innerWidth);
	let stageHeight = $state(window.innerHeight);
	let scroller = $state<HTMLElement>();
	let pagedView = $state<HTMLElement>();
	let fitWidth = $derived(prefs.fit === 'width');
	// 条漫总是上下连续滚动，不管全局的阅读方式。
	const detectedWebtoon = untrack(() => data.webtoon);
	let webtoon = $derived(prefs.webtoon[book.id] ?? detectedWebtoon);
	let mode = $derived(webtoon ? 'scroll' : prefs.mode);
	// 滚动视图那一栏的最大宽度。条漫在电脑和 iPad 横屏上用设置里的宽度，两侧留黑；
	// 竖屏（手机、iPad 竖放）占满宽度。
	let columnWidth = $derived(
		!webtoon ? SCROLL_MAX_WIDTH : innerHeight > innerWidth ? Infinity : prefs.webtoonWidth
	);

	// 双页只在横屏时生效，竖屏手机上一屏放两页太小。
	let spreads = $derived(
		prefs.spread && innerWidth > innerHeight ? buildSpreads(pages) : pages.map((_, index) => [index])
	);
	let spreadIndex = $derived(Math.max(spreads.findIndex((spread) => spread.includes(current)), 0));
	let visible = $derived(spreads[spreadIndex] ?? []);
	let rtlPaged = $derived(mode === 'paged' && prefs.direction === 'rtl');
	let pageLabel = $derived(
		mode === 'paged' && visible.length === 2 ? `${visible[0] + 1}-${visible[1] + 1}` : `${current + 1}`
	);

	// 从书库或详情页进来时用浏览器返回，书库的滚动位置才会恢复。
	let cameFromList = false;
	afterNavigate(({ from }) => {
		cameFromList = from?.route.id === '/' || from?.route.id === '/a/[id]';
	});

	function back(event: MouseEvent) {
		if (!cameFromList) return;
		event.preventDefault();
		history.back();
	}

	// iPhone 的 Safari 不支持网页元素全屏，这时只收起工具栏。
	const canFullscreen = typeof document.documentElement.requestFullscreen === 'function';

	function enterFullscreen() {
		immersive = true;
		overlay = false;
		if (canFullscreen) {
			document.documentElement.requestFullscreen().catch(() => {
				// 浏览器拒绝（例如不是由点击触发）时保持只收起工具栏。
			});
		}
	}

	function exitFullscreen() {
		immersive = false;
		overlay = false;
		if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
	}

	// 用 Esc 或系统手势退出浏览器全屏时，工具栏跟着回来。
	$effect(() => {
		if (!canFullscreen) return;
		const onChange = () => {
			if (!document.fullscreenElement) {
				immersive = false;
				overlay = false;
			}
		};
		document.addEventListener('fullscreenchange', onChange);
		return () => {
			document.removeEventListener('fullscreenchange', onChange);
			if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
		};
	});

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

	// 翻过最后一页时提示下一卷。阅读器的状态在打开时读一次，换书要整页重新加载。
	const nextVolume = book.series?.next ?? null;
	let finishedVolume = $state(false);

	function step(delta: number) {
		const target = spreads[spreadIndex + delta];
		if (target) go(target[0]);
		// 双页时最后一屏只记它的第一页，翻过最后一屏才算读完，补记最后一页，
		// “读完”筛选和系列页的续读才认。
		if (!target && delta > 0 && pages.length > 0) go(pages.length - 1);
		finishedVolume = !target && delta > 0 && nextVolume !== null;
	}

	function src(index: number, cssWidth: number) {
		return pageUrl(pages[index], widthTier(cssWidth, devicePixelRatio));
	}

	// 双页时每页只占半个屏幕宽。适应宽度时页面比屏幕高的部分靠上下滚动，不受屏幕高度限制。
	function pagedSrc(index: number, count: number) {
		const viewport = { width: stageWidth / count, height: stageHeight };
		return src(index, fitWidth ? viewport.width : fittedWidth(pages[index], viewport));
	}

	// 适应宽度时翻到新的一屏从顶部开始看。
	$effect(() => {
		void spreadIndex;
		if (pagedView) pagedView.scrollTop = 0;
	});

	/** 适应宽度时在这一屏里滚动一段；已经到头、或者整页模式时返回 false。 */
	function scrollWithin(delta: 1 | -1, fraction: number): boolean {
		const view = pagedView;
		if (!fitWidth || !view) return false;
		const room = delta > 0 ? view.scrollHeight - view.clientHeight - view.scrollTop : view.scrollTop;
		if (room <= 1) return false;
		view.scrollBy({ top: delta * view.clientHeight * fraction });
		return true;
	}

	// 空格和 PageDown 先把这一屏看完再翻页。
	function advance(delta: 1 | -1) {
		if (!scrollWithin(delta, 0.9)) step(delta);
	}

	// 预取前后的几屏，翻页时直接从 HTTP 缓存取图。
	$effect(() => {
		if (mode !== 'paged') return;
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

	// 两侧三分之一翻页，中间在全屏时叫出或收起底栏。
	function tap(event: MouseEvent) {
		const x = event.clientX / stageWidth;
		if (x > 1 / 3 && x < 2 / 3) {
			if (immersive) overlay = !overlay;
			return;
		}
		const forward = x >= 2 / 3 === (prefs.direction === 'ltr');
		step(forward ? 1 : -1);
	}

	function key(event: KeyboardEvent) {
		if (event.key === 'Escape') {
			// 浏览器全屏时 Esc 由浏览器处理，这里只管面板和只收起工具栏的情况。
			if (panel) panel = null;
			else if (immersive && !document.fullscreenElement) exitFullscreen();
			return;
		}
		if (panel || event.defaultPrevented) return;
		// 焦点在按钮、滑块这些控件上时，空格和方向键是它们自己的操作。
		const target = event.target as Element | null;
		if (target?.closest('button, a, input, select, textarea, [contenteditable]')) return;
		if (mode === 'scroll') {
			// 滚动视图没有焦点，方向键和空格不会自己滚它。
			const keys: Record<string, () => void> = {
				' ': () => scrollScreen(event.shiftKey ? -1 : 1),
				PageDown: () => scrollScreen(1),
				PageUp: () => scrollScreen(-1),
				ArrowDown: () => scroller?.scrollBy({ top: scroller.clientHeight * 0.2 }),
				ArrowUp: () => scroller?.scrollBy({ top: -scroller.clientHeight * 0.2 })
			};
			const action = keys[event.key];
			if (!action) return;
			action();
			event.preventDefault();
			return;
		}
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
				advance(event.shiftKey && event.key === ' ' ? -1 : 1);
				break;
			case 'PageUp':
				advance(-1);
				break;
			case 'ArrowDown':
				if (!fitWidth) return;
				scrollWithin(1, 0.2);
				break;
			case 'ArrowUp':
				if (!fitWidth) return;
				scrollWithin(-1, 0.2);
				break;
			default:
				return;
		}
		event.preventDefault();
	}

	// 滚动模式下中线所在的页，面板关上时如果和当前页不同就补记。
	let middle: number | null = null;
	$effect(() => {
		if (panel === null && mode === 'scroll' && middle !== null && middle !== untrack(() => current)) {
			record(middle);
		}
	});

	// 页面中线所在的那一页算当前页。
	function trackScroll(node: HTMLElement) {
		const observer = new IntersectionObserver(
			(entries) => {
				for (const entry of entries) {
					const index = Number((entry.target as HTMLElement).dataset.index);
					if (!entry.isIntersecting) continue;
					middle = index;
					// 面板开着时（例如从书库只打开详情）不上报，关上面板后再补记。
					if (!panel && index !== current) record(index);
				}
			},
			{ root: node, rootMargin: '-50% 0px -50% 0px' }
		);
		for (const child of node.children) observer.observe(child);
		// 每页都按宽高比占好位置，图片还没加载也能直接滚到目标页。
		node.children[untrack(() => current)]?.scrollIntoView();
		return () => observer.disconnect();
	}

	/**
	 * 滚动视图里跳到下一张或上一张图的顶端。按现在的滚动位置找，不按当前页：当前页按屏幕
	 * 中线判定，一屏有好几张短图时从当前页往前数会原地不动。
	 */
	function scrollToImage(direction: 1 | -1) {
		const view = scroller;
		if (!view) return;
		const top = view.scrollTop;
		const images = [...view.children] as HTMLElement[];
		const target =
			direction > 0
				? images.find((image) => image.offsetTop > top + 1)
				: images.findLast((image) => image.offsetTop < top - 1);
		if (target) view.scrollTo({ top: target.offsetTop });
	}

	// 空格和 PageDown 在滚动视图里滚一屏，留一点上一屏的底边方便接着读。
	function scrollScreen(direction: 1 | -1) {
		scroller?.scrollBy({ top: direction * scroller.clientHeight * 0.9 });
	}

	/** 工具栏上的左右按钮和两侧点击一致：从右往左读时左边是下一页。 */
	function stepLeft() {
		if (mode === 'scroll') scrollToImage(-1);
		else step(prefs.direction === 'rtl' ? 1 : -1);
	}

	function stepRight() {
		if (mode === 'scroll') scrollToImage(1);
		else step(prefs.direction === 'rtl' ? -1 : 1);
	}

	// 滚动模式下明确选了某页就直接记下：整本放得进一屏时滚不动，观察器不会再报。
	function jump(page: number) {
		if (mode !== 'scroll') {
			go(spreads.find((spread) => spread.includes(page))?.[0] ?? page);
			return;
		}
		scroller?.children[page]?.scrollIntoView();
		middle = page;
		go(page);
	}

	function update(change: () => void) {
		change();
		prefs.save();
	}

	/** `undefined` 表示回到按宽高比自动判断。 */
	function setWebtoon(value: boolean | undefined) {
		update(() => {
			const { [book.id]: _, ...others } = prefs.webtoon;
			prefs.webtoon = value === undefined ? others : { ...others, [book.id]: value };
		});
	}
</script>

<svelte:head>
	<title>{book.title} · Shuku</title>
</svelte:head>

<svelte:window bind:innerWidth bind:innerHeight bind:devicePixelRatio onkeydown={key} />

<div class="reader" class:immersive>
	{#if !immersive}
		<header class="top">
			<a class="bar-button" href="/" onclick={back} aria-label="返回书库"><Icon name="back" /></a>
			<h1 title={book.title}>{book.title}</h1>
			{#if pages.length > 0}
				<button class="bar-button" onclick={stepLeft} aria-label={rtlPaged ? '下一页' : '上一页'}>
					<Icon name="chevron-left" />
				</button>
				<button class="counter" onclick={() => (panel = 'detail')} aria-label="详情和页面">
					{pageLabel} / {pages.length}
				</button>
				<button class="bar-button" onclick={stepRight} aria-label={rtlPaged ? '上一页' : '下一页'}>
					<Icon name="chevron-right" />
				</button>
			{/if}
			<BookmarkButton archive={book.id} {members} bind:categories={categoryList} bar />
			<button class="bar-button" onclick={() => (panel = 'detail')} aria-label="详情">
				<Icon name="overview" />
			</button>
			<button class="bar-button" onclick={() => (panel = 'settings')} aria-label="阅读设置">
				<Icon name="settings" />
			</button>
			<!-- 空书没有能点的图片区域，全屏后在 iPhone 上就叫不回工具栏。 -->
			{#if pages.length > 0}
				<button class="bar-button" onclick={enterFullscreen} aria-label="全屏">
					<Icon name="maximize" />
				</button>
			{/if}
		</header>
	{/if}

	<div class="stage" bind:clientWidth={stageWidth} bind:clientHeight={stageHeight}>
		{#if pages.length === 0}
			<p class="empty">这本书没有可显示的页面。</p>
		{:else if mode === 'paged'}
			<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
			<div
				class="paged"
				class:rtl={prefs.direction === 'rtl'}
				class:fit-width={fitWidth}
				bind:this={pagedView}
				onclick={tap}
			>
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
			<div
				class="scroll"
				style:--column={Number.isFinite(columnWidth) ? `${columnWidth}px` : 'none'}
				bind:this={scroller}
				onclick={() => {
					if (immersive) overlay = !overlay;
				}}
				{@attach trackScroll}
			>
				{#each pages as page, index (index)}
					<img
						data-index={index}
						src={src(index, Math.min(stageWidth, columnWidth))}
						width={page.width}
						height={page.height}
						loading={Math.abs(index - current) <= 1 ? 'eager' : 'lazy'}
						alt="第 {index + 1} 页"
					/>
				{/each}
			</div>
		{/if}
	</div>

	<!-- 全屏时不显示任何控件；点屏幕中间出现一条底栏，可以拖动跳页或退出全屏。 -->
	{#if immersive && overlay}
		<footer class="bar">
			<button class="bar-button" onclick={exitFullscreen} aria-label="退出全屏">
				<Icon name="minimize" />
			</button>
			<input
				type="range"
				min="0"
				max={Math.max(pages.length - 1, 0)}
				value={current}
				dir={rtlPaged ? 'rtl' : 'ltr'}
				onchange={(event) => jump(Number(event.currentTarget.value))}
				aria-label="跳到页"
			/>
			<span class="counter">{pageLabel} / {pages.length}</span>
			<button class="bar-button" onclick={() => (panel = 'detail')} aria-label="详情">
				<Icon name="overview" />
			</button>
		</footer>
	{/if}

	{#if panel}
		<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
		<div class="scrim" onclick={() => (panel = null)}></div>
		<aside
			class="panel"
			class:wide={panel === 'detail'}
			aria-label={panel === 'detail' ? '详情' : '阅读设置'}
		>
			<div class="panel-head">
				<h2>{panel === 'detail' ? book.title : '阅读设置'}</h2>
				<button class="bar-button" onclick={() => (panel = null)} aria-label="关闭">
					<Icon name="close" />
				</button>
			</div>
			{#if panel === 'settings'}
				<div class="settings">
					<span>条漫</span>
					<div class="segmented">
						<button class:on={prefs.webtoon[book.id] === undefined} onclick={() => setWebtoon(undefined)}
							>自动（{detectedWebtoon ? '是' : '否'}）</button
						>
						<button class:on={prefs.webtoon[book.id] === true} onclick={() => setWebtoon(true)}>开</button>
						<button class:on={prefs.webtoon[book.id] === false} onclick={() => setWebtoon(false)}>关</button>
					</div>
					{#if webtoon}
						<span>条漫宽度</span>
						<label class="width">
							<input
								type="range"
								min={WEBTOON_WIDTH.min}
								max={WEBTOON_WIDTH.max}
								step={WEBTOON_WIDTH.step}
								value={prefs.webtoonWidth}
								oninput={(event) =>
									update(() => (prefs.webtoonWidth = Number(event.currentTarget.value)))}
								aria-label="条漫宽度"
							/>
							<span>{prefs.webtoonWidth}px</span>
						</label>
					{:else}
						<span>阅读方式</span>
						<div class="segmented">
							<button class:on={prefs.mode === 'paged'} onclick={() => update(() => (prefs.mode = 'paged'))}
								>翻页</button
							>
							<button class:on={prefs.mode === 'scroll'} onclick={() => update(() => (prefs.mode = 'scroll'))}
								>滚动</button
							>
						</div>
					{/if}
					{#if mode === 'paged'}
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
						<span>缩放</span>
						<div class="segmented">
							<button class:on={prefs.fit === 'screen'} onclick={() => update(() => (prefs.fit = 'screen'))}
								>整页</button
							>
							<button class:on={prefs.fit === 'width'} onclick={() => update(() => (prefs.fit = 'width'))}
								>适应宽度</button
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
			{:else}
				<div class="actions">
					<BookmarkButton archive={book.id} {members} bind:categories={categoryList} />
					<a href="/a/{book.id}">相关作品</a>
					{#if book.series}
						<a href="/s/{book.series.id}">{book.series.name}的全部卷</a>
					{/if}
					{#if nextVolume !== null}
						<a href="/a/{nextVolume}/read" data-sveltekit-reload>下一卷</a>
					{/if}
					<a href={downloadUrl(book.id)} download>下载</a>
				</div>
				<CategoryPicker archive={book.id} bind:categories={categoryList} {members} />
				<TagTable rows={tagRows(book)} />
				{#if book.summary}<p class="summary">{book.summary}</p>{/if}
				<div class="overview-pages">
					<PageGrid
						archive={book.id}
						{pages}
						{current}
						onpick={(page) => {
							panel = null;
							jump(page);
						}}
					/>
				</div>
			{/if}
		</aside>
	{/if}

	{#if progressError}
		<p class="toast">进度没有保存：{progressError}</p>
	{/if}

	<!-- 滚动模式没有“再翻一页”，滚到最后一页就提示。 -->
	{#if nextVolume !== null && (finishedVolume || (mode === 'scroll' && current === pages.length - 1))}
		<p class="next-volume" role="status">
			这一卷读完了
			<a href="/a/{nextVolume}/read" data-sveltekit-reload>下一卷</a>
			{#if finishedVolume}
				<button class="bar-button" onclick={() => (finishedVolume = false)} aria-label="关闭">
					<Icon name="close" />
				</button>
			{/if}
		</p>
	{/if}
</div>

<style>
	.reader {
		position: fixed;
		inset: 0;
		display: flex;
		flex-direction: column;
		background: #000;
		color: #eee;
		user-select: none;
		-webkit-user-select: none;
	}

	.top {
		display: flex;
		flex: none;
		align-items: center;
		gap: 0.25rem;
		padding: 0.3rem 0.5rem;
		padding-top: max(0.3rem, env(safe-area-inset-top));
		background: #2b2c31;
		border-bottom: 1px solid #43464e;
	}

	.top h1 {
		flex: 1;
		min-width: 0;
		margin: 0 0.4rem;
		overflow: hidden;
		font-size: 0.95rem;
		font-weight: 500;
		white-space: nowrap;
		text-overflow: ellipsis;
	}

	/* 320px 宽的手机上要放下六个按钮和三位数页码，按钮收窄、标题让位。 */
	@media (max-width: 400px) {
		.top {
			gap: 0;
			padding-inline: 0.25rem;
		}

		.top .bar-button,
		.top :global(.bookmark) {
			width: 2rem;
		}

		.top h1 {
			margin: 0 0.2rem;
		}
	}

	@media (max-width: 360px) {
		.top h1 {
			visibility: hidden;
		}
	}

	/* 图片区域占满工具栏以下的部分，翻页和滚动视图都相对它定位。 */
	.stage {
		position: relative;
		flex: 1;
		min-height: 0;
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

	/* 上下的 auto 外边距让比屏幕矮的页居中，比屏幕高的页从顶部开始。 */
	.paged.fit-width {
		align-items: flex-start;
		overflow-y: auto;
		overscroll-behavior: contain;
	}

	.paged.fit-width .page {
		width: 100%;
		max-width: none;
		max-height: none;
		margin: auto 0;
	}

	.paged.fit-width .page.half {
		width: 50%;
	}

	/* 定位后图片的 offsetTop 相对这个滚动容器，跳到上一张、下一张时按它算。 */
	.scroll {
		position: relative;
		height: 100%;
		overflow-y: auto;
		overscroll-behavior: contain;
	}

	.scroll img {
		display: block;
		width: 100%;
		max-width: var(--column, 960px);
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
		padding: 0.3rem 0.4rem;
		border: none;
		background: transparent;
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

	/* 竖屏时条漫总是占满宽度，这个宽度只在横屏生效。 */
	.width {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		font-variant-numeric: tabular-nums;
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
		--accent: #3b97ea;
		user-select: text;
		-webkit-user-select: text;
	}

	.panel.wide {
		width: min(48rem, 100%);
	}

	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
		margin-bottom: 1rem;
	}

	.actions a {
		padding: 0.4rem 0.8rem;
		border: 1px solid #50545c;
		border-radius: 6px;
		color: #ddd;
		font-size: 0.85rem;
		text-decoration: none;
	}

	.overview-pages {
		margin-top: 1.25rem;
	}

	@media (max-width: 599px) {
		.panel {
			top: auto;
			width: 100%;
			max-height: 75%;
			border-radius: 12px 12px 0 0;
			padding-bottom: max(1rem, env(safe-area-inset-bottom));
		}

		.panel.wide {
			max-height: 90%;
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

	.next-volume {
		position: absolute;
		left: 50%;
		bottom: 5.5rem;
		display: flex;
		align-items: center;
		gap: 0.6rem;
		transform: translateX(-50%);
		margin: 0;
		padding: 0.3rem 0.3rem 0.3rem 0.9rem;
		border: 1px solid #43464e;
		border-radius: 8px;
		background: #2b2c31;
		font-size: 0.9rem;
		white-space: nowrap;
	}

	.next-volume a {
		padding: 0.3rem 0.8rem;
		border-radius: 6px;
		background: #3b97ea;
		color: white;
		text-decoration: none;
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
