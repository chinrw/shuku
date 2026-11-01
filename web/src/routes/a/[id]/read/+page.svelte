<script lang="ts">
	import { onMount, tick, untrack } from 'svelte';
	import { afterNavigate, pushState, replaceState } from '$app/navigation';
	import { page as route } from '$app/state';
	import { SvelteSet } from 'svelte/reactivity';
	import { downloadUrl, putProgress, type Progress } from '#lib/api.ts';
	import BookmarkButton from '#lib/BookmarkButton.svelte';
	import CategoryPicker from '#lib/CategoryPicker.svelte';
	import FinishedButton from '#lib/FinishedButton.svelte';
	import { noteFinished } from '#lib/finished.svelte.ts';
	import Icon from '#lib/Icon.svelte';
	import { bytes } from '#lib/format.ts';
	import { fade, panelEdge, slide, turn } from '#lib/motion.ts';
	import PageGrid from '#lib/PageGrid.svelte';
	import RelatedRow from '#lib/RelatedRow.svelte';
	import { ProgressReporter } from '#lib/progress.ts';
	import { MOTIONS, settings, WEBTOON_WIDTH } from '#lib/settings.svelte.ts';
	import { buildSpreads } from '#lib/spreads.ts';
	import TagTable from '#lib/TagTable.svelte';
	import { tagRows } from '#lib/tags.ts';
	import ThemePicker from '#lib/ThemePicker.svelte';
	import { WheelTurner } from '#lib/wheelTurn.ts';
	import { NO_ZOOM, panBy, zoomAt, type Zoom } from '#lib/zoom.ts';

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
	type Panel = 'detail' | 'settings';
	let panel = $state<Panel | null>(null);
	let detailPages = $state<HTMLElement>();

	// 打开面板时加一条历史记录，手机的返回手势和浏览器的后退只关面板，不离开阅读器。
	function openPanel(kind: Panel) {
		if (panel === kind) return;
		if (panel === null) pushState('', { panel: kind });
		else replaceState('', { panel: kind });
		panel = kind;
	}

	// 关面板时退掉那条历史记录；`then` 等退完再做，免得翻页写的地址被后退冲掉。
	let afterClose: (() => void) | null = null;
	function closePanel(then?: () => void) {
		if (panel === null) {
			then?.();
			return;
		}
		panel = null;
		if (route.state.panel) {
			afterClose = then ?? null;
			history.back();
		} else {
			then?.();
		}
	}

	// 面板跟着历史记录走：后退弹掉面板那条记录（返回手势、浏览器后退，或者 closePanel
	// 自己退的）时关上，前进回到那条记录时再打开。只在记录里的状态变了时同步，
	// openPanel 自己先改了 panel 再压记录，不会被这里关掉。
	let panelInHistory: Panel | undefined = undefined;
	$effect(() => {
		const state = route.state.panel;
		if (state === panelInHistory) return;
		panelInHistory = state;
		untrack(() => {
			panel = state ?? null;
			if (state !== undefined) return;
			const then = afterClose;
			afterClose = null;
			then?.();
		});
	});

	// 书库抽屉的“详情”按钮打开阅读器时直接弹出详情面板。
	onMount(() => {
		if (untrack(() => data.detail)) openPanel('detail');
	});

	// 手机上详情面板分成三页（页面、信息、相似），一次只显示一块；宽屏上三块同时显示。
	type DetailTab = 'pages' | 'info' | 'similar';
	let detailTab = $state<DetailTab>('pages');
	const DETAIL_TABS: [DetailTab, string][] = [
		['pages', '页面'],
		['info', '信息'],
		['similar', '相似']
	];

	// 详情面板里的页面缩略图停在正在看的那一页，不从第一页开始。
	$effect(() => {
		void detailTab;
		if (panel !== 'detail' || !detailPages) return;
		const box = detailPages;
		tick().then(() => box.querySelector('[aria-current="page"]')?.scrollIntoView({ block: 'center' }));
	});
	let progressError = $state<string | null>(null);
	// 关闭了 SSR，这里一定在浏览器里运行。先取一次真实值，免得第一次渲染按宽度 0 请求图片。
	let innerWidth = $state(window.innerWidth);
	let innerHeight = $state(window.innerHeight);
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

	// 手机和 iPad 上可以把整个阅读器（连工具栏和面板）转 90 度看横跨两页的图，不用转设备方向锁。
	const touchDevice = window.matchMedia('(pointer: coarse)').matches;
	let rotated = $state(false);

	// 双页只在横屏时生效，竖屏手机上一屏放两页太小。页面转了 90 度时横竖对调。
	let landscape = $derived(rotated ? innerHeight > innerWidth : innerWidth > innerHeight);
	// 滚动视图那一栏的最大宽度。条漫在电脑和 iPad 横屏上用设置里的宽度，两侧留黑；
	// 竖屏（手机、iPad 竖放）占满宽度。
	let columnWidth = $derived(
		!webtoon ? SCROLL_MAX_WIDTH : !landscape ? Infinity : prefs.webtoonWidth
	);

	let spreads = $derived(
		prefs.spread && landscape ? buildSpreads(pages) : pages.map((_, index) => [index])
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
		// 阅读器里的历史前进后退（关面板时退掉的那条记录）不算换了来源，保留进来时的判断。
		if (from?.route.id === '/a/[id]/read') return;
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
	// 这本书最新的进度：打开时来自直接读库的详情接口，之后按上报和“标记读完”的回复更新。
	// 读完标记也记进 finished.svelte.ts，回到书库时角标不等快照刷新就是对的。
	let latestProgress = $state(untrack(() => book.progress));
	function noteProgress(progress: Progress | null) {
		latestProgress = progress;
		noteFinished([book.id], progress?.finished ?? false);
	}
	untrack(() => noteProgress(book.progress));
	const reporter = new ProgressReporter(
		book.id,
		(error) => {
			progressError = errorText(error);
		},
		async (id, progress, options) => {
			const result = await putProgress(id, progress, options);
			noteProgress(result.progress);
			return result;
		}
	);

	// 只在翻页时上报。打开时不报，免得用旧页码和新时间戳盖掉别的设备更新的进度。
	function record(page: number) {
		current = page;
		progressError = null;
		reporter.turn(page);
		// 保留浅层路由的状态，面板开着时换页不会把面板那条历史记录冲掉。
		replaceState(`?page=${page}`, route.state);
	}

	// 翻页模式下点两侧、滑动、方向键这样一屏一屏地翻时，新的一屏滑进来，方向和阅读方向一致：
	// 从右往左读时下一屏从左边进来。跳页（滑块、缩略图、第一页和最后一页）直接换。
	let turnSign = $state(0);
	// 每次带动画的翻页都换一个新的元素：快速来回翻时 Svelte 会复用还在滑出去的那一屏，
	// 它保留着旧的滚动位置和缓存的滑动方向。
	let turns = $state(0);

	function go(page: number, delta = 0) {
		if (page < 0 || page >= pages.length || page === current) return;
		turnSign = delta * (prefs.direction === 'rtl' ? -1 : 1);
		if (delta) turns += 1;
		record(page);
	}

	// 翻过最后一页时提示下一卷。阅读器的状态在打开时读一次，换书要整页重新加载。
	const nextVolume = book.series?.next ?? null;
	let finishedVolume = $state(false);

	function step(delta: number) {
		const target = spreads[spreadIndex + delta];
		if (target) go(target[0], delta);
		// 双页时最后一屏只记它的第一页，翻过最后一屏才算读完，补记最后一页，
		// “读完”筛选和系列页的续读才认。
		if (!target && delta > 0 && pages.length > 0) go(pages.length - 1);
		finishedVolume = !target && delta > 0 && nextVolume !== null;
	}

	// 手机和平板默认按显示需要的宽度取服务端缩小的 JPEG：几千万像素的原图在手机上要解码一两秒、
	// 占上百 MB 内存，翻页会卡。只用 1440 和 2160 两档，比屏幕清晰度略高，稍微放大也不糊；
	// 真正放大过的页换成原图。电脑上照旧用原图，原来按屏幕请求时整页显示只有 720 宽，放大看会糊。
	// 动图由服务端原样返回，超过服务端缩放上限（6400 万像素）的和坏页直接用原图。
	const VARIANT_WIDTHS = [1440, 2160];
	const MAX_VARIANT_PIXELS = 64_000_000;
	let variants = $derived(touchDevice && prefs.quality === 'auto');
	// 放大看过的页，之后一直用原图。原图加载失败（例如离线时只缓存过缩小的图）的页不再换。
	const upgraded = new SvelteSet<number>();
	const keepVariant = new Set<number>();

	/** 这一页在当前布局下显示的宽度（CSS 像素）。 */
	function displayWidth(index: number): number {
		if (mode === 'scroll') return Math.min(stageWidth, columnWidth);
		const across = stageWidth / (spreads.find((spread) => spread.includes(index))?.length ?? 1);
		if (fitWidth) return across;
		const { width, height } = pages[index];
		return Math.min(across, (stageHeight * width) / height);
	}

	/**
	 * 整页模式里图片的 CSS 宽度。比屏幕小的图也放大到占满（全屏后跟着变大），只用 CSS 的
	 * max-width 和 max-height 时只缩不放。适应宽度时由 CSS 占满宽度；坏页不知道宽高比。
	 */
	function fittedWidth(index: number): string | null {
		const { width, height } = pages[index];
		if (fitWidth || width === 0 || height === 0) return null;
		return `${Math.floor(displayWidth(index))}px`;
	}

	/** 按屏幕该取的缩小的图；不该缩（电脑、设成原图、图本来就不大、坏页）时是 null。 */
	function variantUrl(index: number): string | null {
		const page = pages[index];
		if (!variants || page.width === 0) return null;
		if (page.width * page.height > MAX_VARIANT_PIXELS) return null;
		const needed = displayWidth(index) * window.devicePixelRatio;
		const width = VARIANT_WIDTHS.find((tier) => tier >= needed);
		return width && width < page.width ? `${page.url}&w=${width}` : null;
	}

	function src(index: number) {
		return (!upgraded.has(index) && variantUrl(index)) || pages[index].url;
	}

	// 两指放大（viewport 的缩放）时把眼前的页换成原图；换 src 时旧图一直显示到新图加载完，
	// 翻页模式下缩小的图还垫在背景里，原图解码完之前不会空。放大着点两侧翻页时新的一屏也换。
	// 滚动模式按当前页和前后各一页算。
	function upgradeIfZoomed() {
		if (!variants || (window.visualViewport?.scale ?? 1) <= 1.05) return;
		const shown = mode === 'paged' ? visible : [current - 1, current, current + 1];
		for (const index of shown) {
			if (pages[index] && !keepVariant.has(index) && variantUrl(index)) upgraded.add(index);
		}
	}

	$effect(() => {
		const viewport = window.visualViewport;
		if (!viewport || !variants) return;
		viewport.addEventListener('resize', upgradeIfZoomed);
		return () => viewport.removeEventListener('resize', upgradeIfZoomed);
	});

	$effect(() => {
		void visible;
		void current;
		untrack(upgradeIfZoomed);
	});

	// 浏览器解不了原图的格式（例如 Chrome 不支持的 JPEG XL）时，改要服务端转出的 JPEG：
	// 取比原图窄的最宽一档，服务端只缩不放，不比原图窄就会原样返回。
	const JPEG_WIDTHS = [2160, 1440, 1080, 720];

	// 每个 img 记下失败过的地址，依次试原图、按屏幕取的图、转出的 JPEG，每个地址只试一次。
	// 按屏幕取的图失败时（服务端缩不了、离线时只缓存过原图）因此会退回原图。
	function fallback(event: Event, index: number) {
		const image = event.currentTarget as HTMLImageElement;
		const page = pages[index];
		const failed = new Set(image.dataset.failed?.split('\n'));
		failed.add(image.getAttribute('src') ?? '');
		image.dataset.failed = [...failed].join('\n');
		const variant = variantUrl(index);
		// 放大时换的原图失败了，退回缩小的图：去掉标记后 src() 重新算出它的地址。
		if (upgraded.has(index) && variant && !failed.has(variant)) {
			upgraded.delete(index);
			keepVariant.add(index);
			return;
		}
		const width = JPEG_WIDTHS.find((tier) => tier < page.width);
		const jpeg = width ? `${page.url}&w=${width}` : null;
		const next = [page.url, variant, jpeg].find((url) => url && !failed.has(url));
		if (next) image.src = next;
	}

	// 非全屏时工具栏上显示的当前页分辨率和文件大小，和 LRR 一样；双页时两页都写。
	let pageInfo = $derived(
		(mode === 'paged' ? visible : [current])
			.filter((index) => pages[index])
			.map((index) => `${pages[index].width}×${pages[index].height} · ${bytes(pages[index].size)}`)
			.join(' | ')
	);

	let stage = $state<HTMLElement>();

	/**
	 * 正在显示的那一屏。翻页动画时旧的一屏还在 DOM 里滑出去，Svelte 把它设成 inert。
	 * 每一屏自己滚动（适应宽度时），换屏总是新建元素，所以新的一屏从顶部开始。
	 */
	function shownSpread(): HTMLElement | null {
		return pagedView?.querySelector<HTMLElement>(':scope > .spread:not([inert])') ?? null;
	}

	/** 适应宽度时在这一屏里滚动一段；已经到头、或者整页模式时返回 false。 */
	function scrollWithin(delta: 1 | -1, fraction: number): boolean {
		const view = shownSpread();
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

	/**
	 * 一张图加载完（或彻底失败）时兑现。原图解不了时 fallback() 在 error 事件里换成服务端的
	 * JPEG，这时图片又变回“未完成”，接着等换上的那张。
	 */
	function settled(image: HTMLImageElement): Promise<void> {
		return new Promise((resolve) => {
			const check = () => {
				if (image.complete) {
					resolve();
					return;
				}
				const wait = new AbortController();
				const next = () => {
					wait.abort();
					check();
				};
				image.addEventListener('load', next, { signal: wait.signal });
				image.addEventListener('error', next, { signal: wait.signal });
			};
			check();
		});
	}

	/**
	 * 超过这个像素数的图预取时不解码（约 48 MiB 的 RGBA）。离屏的 `decode()` 按原尺寸解码，
	 * 7000×4700 的图要占 125 MiB，超过 Chrome 的解码缓存预算，翻过去时多半已被淘汰；
	 * 显示时浏览器可以按屏幕尺寸缩小着解码，反而更快。
	 */
	const PREDECODE_PIXELS = 12_000_000;

	// 预取后两屏和前一屏，翻页时直接从 HTTP 缓存取图。原图可能有二十几 MB，一起下载会和
	// 当前这屏抢带宽，所以等当前这屏加载完，再一张一张地取；下一屏不太大的图下载完顺便解码，
	// 翻过去马上能显示，其余的只下载不解码，免得手机上同时占着几张大图解码后的内存。
	// 解码不等，下一张接着下载：等解码完再下载会让网络空着，大图上能把速率拉低一半以上。
	// 按屏幕取的图只有一两 MB，但服务端第一次生成每张要 0.2 到 0.4 秒，所以多取一屏；
	// 要取的全是缩小的图时同时取两张，服务端的几个核一起生成。动图要下载完才知道服务端
	// 原样返回了它，这时两张原图会分着带宽下载，只是下一页晚到一些。翻走时取消还没取完的。
	$effect(() => {
		if (mode !== 'paged') return;
		const next = spreads[spreadIndex + 1] ?? [];
		const ahead = variants ? [2, 3] : [2];
		const queue = [
			...next.map((index) => ({ index, decode: true })),
			...ahead
				.flatMap((offset) => spreads[spreadIndex + offset] ?? [])
				.map((index) => ({ index, decode: false })),
			...(spreads[spreadIndex - 1] ?? []).map((index) => ({ index, decode: false }))
		];
		let cancelled = false;
		const started: HTMLImageElement[] = [];
		let taken = 0;
		const worker = async () => {
			while (!cancelled && taken < queue.length) {
				const { index, decode } = queue[taken++];
				const image = new Image();
				started.push(image);
				image.src = src(index);
				await settled(image);
				// 按下载到的图算：缩小的长条图和服务端原样返回的动图也可能很大。
				const small = image.naturalWidth * image.naturalHeight <= PREDECODE_PIXELS;
				if (decode && !cancelled && small) {
					image.decode().catch(() => {});
				}
			}
		};
		(async () => {
			const shown = [...(shownSpread()?.querySelectorAll('img') ?? [])];
			await Promise.all(shown.map(settled));
			const allVariants = untrack(() => queue.every(({ index }) => src(index) !== pages[index].url));
			await Promise.all(Array.from({ length: allVariants ? 2 : 1 }, worker));
		})();
		return () => {
			cancelled = true;
			// 清掉 src 让浏览器停止还没下载完的请求。
			for (const image of started) if (!image.complete) image.src = '';
		};
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

	/**
	 * 读者眼里的横向位置（0 到 1，左到右）。页面转了 90 度（顺时针）时，读者把设备逆时针
	 * 转过来看，屏幕的上方成了读者的左边，所以按屏幕的纵向算。
	 */
	function across(event: { clientX: number; clientY: number }): number {
		const box = stage?.getBoundingClientRect();
		if (!box) return event.clientX / stageWidth;
		return rotated
			? (event.clientY - box.top) / box.height
			: (event.clientX - box.left) / box.width;
	}

	// 电脑上放大看一屏：Ctrl+滚轮（触控板两指张开时浏览器也发这个）以鼠标为中心缩放，
	// + / - / 0 键缩放和还原，放大后拖动或滚轮平移。只在整页的翻页模式里，翻页时还原。
	// 手机和 iPad（含接了键盘的）用浏览器自己的两指缩放：单指拖动、滑动翻页和放大换原图都按
	// 浏览器的缩放判断。转 90 度也只在触屏设备上有。
	let zoom = $state<Zoom>(NO_ZOOM);
	let zoomable = $derived(!touchDevice && mode === 'paged' && !fitWidth);
	let zoomed = $derived(zoom.scale > 1);
	let drag: { x: number; y: number } | null = $state(null);
	// 这次按下后拖动过，松开时的点击不算点按。
	let panned = false;

	$effect(() => {
		void spreadIndex;
		void zoomable;
		void stageWidth;
		void stageHeight;
		zoom = NO_ZOOM;
	});

	function zoomTo(factor: number, px = stageWidth / 2, py = stageHeight / 2) {
		zoom = zoomAt(zoom, factor, px, py, stageWidth, stageHeight);
	}

	const wheelTurner = new WheelTurner();

	/** 适应宽度时这一屏在 direction 方向上已经滚到头；整页模式总是到头。 */
	function atEdge(direction: number): boolean {
		const view = shownSpread();
		if (!fitWidth || !view) return true;
		const room = direction > 0 ? view.scrollHeight - view.clientHeight - view.scrollTop : view.scrollTop;
		return room <= 1;
	}

	// 滚轮：Ctrl 时缩放，放大着时平移，否则（设置里开着时）往下滚翻到下一页、往上滚翻回上一页。
	// 适应宽度时先在这一屏里滚动，到头后再滚一次才翻页。
	// 监听要能 preventDefault，所以不用 onwheel 属性，自己加一个非 passive 的。
	function wheelZoom(node: HTMLElement) {
		const onWheel = (event: WheelEvent) => {
			// 按行滚动的鼠标（Firefox）每格约 16 像素。
			const unit = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 : 1;
			if (!zoomable) {
				turnByWheel(event, unit);
				return;
			}
			if (event.ctrlKey) {
				event.preventDefault();
				const box = stage?.getBoundingClientRect();
				const px = event.clientX - (box?.left ?? 0);
				const py = event.clientY - (box?.top ?? 0);
				zoomTo(Math.exp(-event.deltaY * unit * 0.002), px, py);
			} else if (zoomed) {
				event.preventDefault();
				// Shift+滚轮横向移动，有的浏览器仍然只给 deltaY。
				const dx = event.shiftKey && event.deltaX === 0 ? event.deltaY : event.deltaX;
				const dy = event.shiftKey && event.deltaX === 0 ? 0 : event.deltaY;
				zoom = panBy(zoom, -dx * unit, -dy * unit, stageWidth, stageHeight);
			} else {
				turnByWheel(event, unit);
			}
		};
		node.addEventListener('wheel', onWheel, { passive: false });
		return () => node.removeEventListener('wheel', onWheel);
	}

	function turnByWheel(event: WheelEvent, unit: number) {
		if (!prefs.wheelTurn || event.ctrlKey || event.shiftKey) return;
		// 接了鼠标或触控板的 iPad 用两指放大后，滚轮留给浏览器平移。
		if ((window.visualViewport?.scale ?? 1) > 1.01) return;
		if (Math.abs(event.deltaX) > Math.abs(event.deltaY)) return;
		const delta = event.deltaY * unit;
		if (!fitWidth) event.preventDefault();
		const discrete = event.deltaMode !== WheelEvent.DOM_DELTA_PIXEL;
		const turn = wheelTurner.feed(delta, event.timeStamp, atEdge(delta), discrete);
		if (turn !== 0) step(turn);
	}

	function pointerDown(event: PointerEvent) {
		panned = false;
		if (!zoomed || event.pointerType !== 'mouse' || event.button !== 0) return;
		drag = { x: event.clientX, y: event.clientY };
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
	}

	function pointerMove(event: PointerEvent) {
		if (!drag) return;
		const dx = event.clientX - drag.x;
		const dy = event.clientY - drag.y;
		if (Math.abs(dx) + Math.abs(dy) > 0) panned = true;
		zoom = panBy(zoom, dx, dy, stageWidth, stageHeight);
		drag = { x: event.clientX, y: event.clientY };
	}

	// 两侧三分之一翻页，中间在全屏时叫出或收起底栏。刚滑动翻过页时浏览器可能还补一次点击，忽略它。
	// 放大时点两侧不翻页，免得拖动看图时误翻。
	function tap(event: MouseEvent) {
		if (swiped || panned) return;
		const x = across(event);
		if (zoomed && (x <= 1 / 3 || x >= 2 / 3)) return;
		if (x > 1 / 3 && x < 2 / 3) {
			if (immersive) overlay = !overlay;
			return;
		}
		const forward = x >= 2 / 3 === (prefs.direction === 'ltr');
		step(forward ? 1 : -1);
	}

	// 手指左右滑动翻页，方向和点两侧一致：从右往左读时往右滑是下一页。两指缩放、
	// 页面已经放大（这时单指是拖动）或者主要是上下滑时不翻页，留给浏览器。
	const SWIPE_DISTANCE = 50;
	const SWIPE_TIME = 800;
	let swipe: { x: number; y: number; time: number; pinch: boolean } | null = null;
	let swiped = false;

	function touchStart(event: TouchEvent) {
		if (event.touches.length > 1) {
			if (swipe) swipe.pinch = true;
			return;
		}
		const touch = event.touches[0];
		swipe = { x: touch.clientX, y: touch.clientY, time: event.timeStamp, pinch: false };
	}

	function touchEnd(event: TouchEvent) {
		const start = swipe;
		if (!start || event.touches.length > 0) return;
		swipe = null;
		if (start.pinch || (window.visualViewport?.scale ?? 1) > 1.01) return;
		const touch = event.changedTouches[0];
		const screenX = touch.clientX - start.x;
		const screenY = touch.clientY - start.y;
		// 转了 90 度时读者的横向是屏幕的纵向，见 across()。
		const dx = rotated ? screenY : screenX;
		const dy = rotated ? -screenX : screenY;
		const horizontal = Math.abs(dx) >= SWIPE_DISTANCE && Math.abs(dx) > Math.abs(dy) * 1.5;
		if (!horizontal || event.timeStamp - start.time > SWIPE_TIME) return;
		swiped = true;
		setTimeout(() => (swiped = false), 400);
		const forward = dx < 0 === (prefs.direction === 'ltr');
		step(forward ? 1 : -1);
	}

	function key(event: KeyboardEvent) {
		if (event.key === 'Escape') {
			// 浏览器全屏时 Esc 由浏览器处理，这里只管面板和只收起工具栏的情况。
			if (panel) closePanel();
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
		if (zoomable && !event.altKey && !event.metaKey) {
			// Ctrl 加这几个键本来是浏览器缩放整个网页，在这里页面只会缩在一屏里不变大，所以接过来。
			const factor: Record<string, number> = { '+': 1.5, '=': 1.5, '-': 1 / 1.5 };
			if (factor[event.key]) {
				zoomTo(factor[event.key]);
				event.preventDefault();
				return;
			}
			if (event.key === '0') {
				zoom = NO_ZOOM;
				event.preventDefault();
				return;
			}
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
				if (zoomed) zoom = panBy(zoom, 0, -stageHeight * 0.2, stageWidth, stageHeight);
				else if (fitWidth) scrollWithin(1, 0.2);
				else return;
				break;
			case 'ArrowUp':
				if (zoomed) zoom = panBy(zoom, 0, stageHeight * 0.2, stageWidth, stageHeight);
				else if (fitWidth) scrollWithin(-1, 0.2);
				else return;
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

	// 图片加载完先解码再显示：`decoding="async"` 时 load 之后还没解码完，直接去掉垫底的
	// 缩略图会空一下。解码期间换了地址（放大换原图、改了设置）时不动，由新地址的 load 负责显示。
	// 放大换上的原图解码完后也撤掉垫在背景里的缩小的图，免得透过透明的地方露出来。
	async function reveal(image: HTMLImageElement) {
		const requested = image.src;
		await image.decode().catch(() => {});
		if (image.src !== requested) return;
		image.classList.add('loaded');
		if (image.classList.contains('upgraded')) image.classList.add('sharp');
	}

	// 预取过、插进来时已经下载完的图不会再触发 load。
	function showIfCached(image: HTMLImageElement) {
		if (image.complete && image.naturalWidth > 0) reveal(image);
	}

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

<svelte:window bind:innerWidth bind:innerHeight onkeydown={key} />

<div class="reader" class:immersive class:rotated>
	{#if !immersive}
		<header class="top">
			<a class="bar-button" href="/" onclick={back} aria-label="返回书库"><Icon name="back" /></a>
			<div class="title">
				<h1 title={book.title}>{book.title}</h1>
				{#if pageInfo}<span class="page-info">{pageInfo}</span>{/if}
			</div>
			{#if pages.length > 0}
				<button
					class="bar-button arrow ends"
					onclick={() => jump(rtlPaged ? pages.length - 1 : 0)}
					aria-label={rtlPaged ? '最后一页' : '第一页'}
				>
					<Icon name="chevrons-left" />
				</button>
				<button class="bar-button arrow" onclick={stepLeft} aria-label={rtlPaged ? '下一页' : '上一页'}>
					<Icon name="chevron-left" />
				</button>
				<button class="counter" onclick={() => openPanel('detail')} aria-label="详情和页面">
					{pageLabel} / {pages.length}
				</button>
				<button class="bar-button arrow" onclick={stepRight} aria-label={rtlPaged ? '上一页' : '下一页'}>
					<Icon name="chevron-right" />
				</button>
				<button
					class="bar-button arrow ends"
					onclick={() => jump(rtlPaged ? 0 : pages.length - 1)}
					aria-label={rtlPaged ? '第一页' : '最后一页'}
				>
					<Icon name="chevrons-right" />
				</button>
			{/if}
			{#if touchDevice && (mode === 'paged' || rotated) && pages.length > 0}
				<button
					class="bar-button"
					class:on={rotated}
					aria-pressed={rotated}
					onclick={() => (rotated = !rotated)}
					aria-label="把页面转 90 度"
				>
					<Icon name="rotate" />
				</button>
			{/if}
			<BookmarkButton archive={book.id} {members} bind:categories={categoryList} bar />
			<button class="bar-button" onclick={() => openPanel('detail')} aria-label="详情">
				<Icon name="overview" />
			</button>
			<button class="bar-button" onclick={() => openPanel('settings')} aria-label="阅读设置">
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

	<div
		class="stage"
		bind:this={stage}
		bind:clientWidth={stageWidth}
		bind:clientHeight={stageHeight}
	>
		{#if pages.length === 0}
			<p class="empty">这本书没有可显示的页面。</p>
		{:else if mode === 'paged'}
			<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
			<div
				class="paged"
				class:rtl={prefs.direction === 'rtl'}
				class:fit-width={fitWidth}
				class:zoomed
				class:dragging={drag !== null}
				style:transform={zoomed ? `translate(${zoom.x}px, ${zoom.y}px) scale(${zoom.scale})` : null}
				bind:this={pagedView}
				onclick={tap}
				ontouchstart={touchStart}
				ontouchend={touchEnd}
				onpointerdown={pointerDown}
				onpointermove={pointerMove}
				onpointerup={() => (drag = null)}
				onpointercancel={() => (drag = null)}
				{@attach wheelZoom}
			>
				<!-- 点击和滑动由外面这层处理，翻页动画播放时照样能接着翻。新旧两屏叠在一起各自滑动。
					转屏等重新排版时不播放：turnSign 在这一屏滑进来以后清零。 -->
				{#key `${visible.join()}:${turns}`}
					<div
						class="spread"
						in:turn={{ sign: turnSign }}
						out:turn={{ sign: turnSign }}
						onintroend={() => (turnSign = 0)}
					>
						{#each visible as index (index)}
							<!-- 大图下载并解码完之前用缩略图垫底；放大换原图时垫的是缩小的图。 -->
							<img
								class="page"
								class:half={visible.length === 2}
								class:upgraded={upgraded.has(index)}
								src={src(index)}
								style:width={fittedWidth(index)}
								style:background-image="url({(upgraded.has(index) && variantUrl(index)) ||
									pages[index].thumbnail_url})"
								onload={(event) => reveal(event.currentTarget as HTMLImageElement)}
								onerror={(event) => fallback(event, index)}
								{@attach showIfCached}
								decoding="async"
								draggable="false"
								width={pages[index].width}
								height={pages[index].height}
								alt="第 {index + 1} 页"
							/>
						{/each}
					</div>
				{/key}
			</div>
			{#if zoomed}
				<button class="zoom-badge" onclick={() => (zoom = NO_ZOOM)} aria-label="还原大小">
					{Math.round(zoom.scale * 100)}%<Icon name="close" />
				</button>
			{/if}
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
						src={src(index)}
						onerror={(event) => fallback(event, index)}
						decoding="async"
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
			<button class="bar-button" onclick={() => openPanel('detail')} aria-label="详情">
				<Icon name="overview" />
			</button>
		</footer>
	{/if}

	{#if panel}
		<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
		<div class="scrim" onclick={() => closePanel()} in:fade out:fade></div>
		<aside
			class="panel"
			in:slide={{ from: panelEdge() }}
			out:slide={{ from: panelEdge() }}
			class:wide={panel === 'detail'}
			data-tab={detailTab}
			aria-label={panel === 'detail' ? '详情' : '阅读设置'}
		>
			<div class="panel-head">
				<h2>{panel === 'detail' ? book.title : '阅读设置'}</h2>
				<button class="bar-button" onclick={() => closePanel()} aria-label="关闭">
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
						<span>滚轮翻页</span>
						<div class="segmented">
							<button class:on={prefs.wheelTurn} onclick={() => update(() => (prefs.wheelTurn = true))}
								>开</button
							>
							<button class:on={!prefs.wheelTurn} onclick={() => update(() => (prefs.wheelTurn = false))}
								>关</button
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
					{#if touchDevice}
						<span>图片</span>
						<div class="segmented">
							<button
								class:on={prefs.quality === 'auto'}
								onclick={() => update(() => (prefs.quality = 'auto'))}
								title="按屏幕取缩小的图，放大时换原图">按屏幕</button
							>
							<button
								class:on={prefs.quality === 'original'}
								onclick={() => update(() => (prefs.quality = 'original'))}>原图</button
							>
						</div>
					{/if}
					<span class="picker-label">配色</span>
					<ThemePicker />
					<span>滑动动画</span>
					<div class="segmented">
						{#each MOTIONS as [key, label] (key)}
							<button class:on={prefs.motion === key} onclick={() => update(() => (prefs.motion = key))}
								>{label}</button
							>
						{/each}
					</div>
				</div>
			{:else}
				<!-- 上面是标签和操作，下面是相似作品，都固定不动；中间的页面缩略图单独滚动。 -->
				<div class="detail-tabs" role="tablist" aria-label="详情的内容">
					{#each DETAIL_TABS as [key, label] (key)}
						<button
							role="tab"
							aria-selected={detailTab === key}
							onclick={() => (detailTab = key)}>{label}</button
						>
					{/each}
				</div>
				<div class="detail-top">
					<div class="actions">
						<BookmarkButton archive={book.id} {members} bind:categories={categoryList} />
						<button onclick={() => closePanel(() => jump(0))}>回到第一页</button>
						<button onclick={() => closePanel(() => jump(pages.length - 1))}>最后一页</button>
						<FinishedButton archive={book.id} progress={latestProgress} onchange={noteProgress} />
						<a href="/a/{book.id}">相关作品</a>
						{#if book.series}
							<a href="/s/{book.series.id}">{book.series.name}的全部卷</a>
						{/if}
						{#if nextVolume !== null}
							<a href="/a/{nextVolume}/read" data-sveltekit-reload>下一卷</a>
						{/if}
						<a href={downloadUrl(book.id)} download>下载</a>
						<a href="/a/{book.id}?edit">编辑</a>
					</div>
					<CategoryPicker archive={book.id} bind:categories={categoryList} {members} />
					<TagTable rows={tagRows(book)} saveable />
					{#if book.summary}<p class="summary">{book.summary}</p>{/if}
				</div>
				<div class="overview-pages" bind:this={detailPages}>
					<PageGrid
						archive={book.id}
						{pages}
						{current}
						onpick={(page) => closePanel(() => jump(page))}
					/>
				</div>
				<!-- 阅读器的状态只在打开时读一次，换到别的书要整页重新加载，和“下一卷”一样。 -->
				<div class="detail-bottom" data-sveltekit-reload>
					<RelatedRow label="相似作品" archive={book.id} compact />
				</div>
				<!-- 手机上“相似”那一页：同样的推荐排成网格，占满整页。 -->
				<div class="detail-similar" data-sveltekit-reload>
					<RelatedRow label="相似作品" archive={book.id} grid />
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
		container: reader / inline-size;
		background: var(--bg);
		color: var(--fg);
		user-select: none;
		-webkit-user-select: none;
	}

	/* 转 90 度：整个阅读器按横屏的宽高排版，再顺时针转过去，工具栏、面板和图一起转。 */
	.reader.rotated {
		inset: auto;
		top: 50%;
		left: 50%;
		width: 100dvh;
		height: 100dvw;
		transform: translate(-50%, -50%) rotate(90deg);
	}

	/* 全屏时四周全黑；平时用和书库一样的底色，不让整页黑着。 */
	.reader.immersive {
		background: #000;
		color: #eee;
	}

	.top {
		display: flex;
		flex: none;
		align-items: center;
		gap: 0.25rem;
		padding: 0.3rem 0.5rem;
		padding-top: max(0.3rem, env(safe-area-inset-top));
		background: var(--sunken);
		border-bottom: 1px solid var(--border);
	}

	.title {
		display: flex;
		flex: 1;
		flex-direction: column;
		min-width: 0;
		margin: 0 0.4rem;
	}

	.title h1,
	.page-info {
		overflow: hidden;
		white-space: nowrap;
		text-overflow: ellipsis;
	}

	.title h1 {
		margin: 0;
		font-size: 0.95rem;
		font-weight: 500;
	}

	.page-info {
		color: var(--muted);
		font-size: 0.72rem;
		font-variant-numeric: tabular-nums;
	}

	.bar-button.on {
		color: var(--accent);
	}

	/* 320px 宽的手机上要放下六个按钮和三位数页码，按钮收窄、标题让位。 */
	/* 下面几条按阅读器自己的宽度算（容器查询），不按浏览器窗口：转 90 度后阅读器的宽度是
	   窗口的高度。 */
	@container reader (max-width: 400px) {
		.top {
			gap: 0;
			padding-inline: 0.25rem;
		}

		.top .bar-button,
		.top :global(.bookmark) {
			width: 2rem;
		}

		.top .title {
			margin: 0 0.2rem;
		}
	}

	@container reader (max-width: 360px) {
		.top .title {
			visibility: hidden;
		}
	}

	/* 窄屏手机上放不下全部按钮。左右翻页按钮让位：手机上点两侧或左右滑动就能翻页，
	   详情按钮要留着（藏起来后只能点页码打开，用户找不到）。 */
	@media (pointer: coarse) {
		@container reader (max-width: 400px) {
			.top .arrow {
				display: none;
			}
		}
	}

	/* 触屏设备上工具栏放不下首尾页按钮，它们在详情面板里也有。 */
	@media (pointer: coarse) {
		.top .ends {
			display: none;
		}
	}

	/* 图片区域占满工具栏以下的部分，翻页和滚动视图都相对它定位。 */
	.stage {
		position: relative;
		flex: 1;
		min-height: 0;
		/* 放大后超出图片区域的部分不能盖到工具栏上。 */
		overflow: hidden;
	}

	/* 翻页动画时新旧两屏叠在这个框里滑动，裁掉露在外面的部分。 */
	.paged {
		position: absolute;
		inset: 0;
		overflow: hidden;
	}

	.spread {
		position: absolute;
		inset: 0;
		display: flex;
		align-items: center;
		justify-content: center;
	}

	.paged {
		transform-origin: 0 0;
	}

	.paged.zoomed {
		cursor: grab;
	}

	.paged.dragging {
		cursor: grabbing;
	}

	.zoom-badge {
		position: absolute;
		right: 0.75rem;
		bottom: 0.75rem;
		display: flex;
		align-items: center;
		gap: 0.3rem;
		padding: 0.3rem 0.5rem 0.3rem 0.7rem;
		border: none;
		border-radius: 999px;
		background: rgb(0 0 0 / 0.65);
		color: #fff;
		font: inherit;
		font-size: 0.85rem;
		font-variant-numeric: tabular-nums;
		cursor: pointer;
	}

	.zoom-badge :global(svg) {
		width: 1rem;
		height: 1rem;
	}

	/* 从右往左读时，第一页放在右边。 */
	.paged.rtl .spread {
		flex-direction: row-reverse;
	}

	.page {
		max-width: 100%;
		max-height: 100%;
		width: auto;
		height: auto;
		object-fit: contain;
		background-position: center;
		background-repeat: no-repeat;
		background-size: contain;
	}

	/* 没加载完时只显示缩略图背景：图片内容移出框外（img 会裁掉框外的内容），
	   免得边下载边一截一截地画出来。 */
	.page:not(:global(.loaded)) {
		object-position: -100000px 0;
	}

	.page:global(.loaded):not(.upgraded),
	.page:global(.sharp) {
		background-image: none !important;
	}

	.page.half {
		max-width: 50%;
	}

	/* 上下的 auto 外边距让比屏幕矮的页居中，比屏幕高的页从顶部开始。 */
	.paged.fit-width .spread {
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
		color: var(--muted);
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
		color: var(--muted);
	}

	/* 跟随系统时配色下面还有两行，标签对齐第一行。 */
	.picker-label {
		align-self: start;
		padding-top: 0.35rem;
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
		border: 1px solid var(--border);
		border-radius: 6px;
		overflow: hidden;
	}

	.segmented button {
		padding: 0.3rem 0.9rem;
		border: none;
		background: transparent;
		color: var(--muted);
		font-size: 0.85rem;
		cursor: pointer;
	}

	.segmented button.on {
		background: var(--chip);
		color: var(--fg);
	}

	.scrim {
		position: absolute;
		inset: 0;
		background: rgb(0 0 0 / 0.45);
	}

	/* 宽屏是右侧抽屉，手机上是底部弹出的面板。 */
	.panel {
		position: absolute;
		top: 0;
		right: 0;
		bottom: 0;
		width: min(26rem, 100%);
		overflow-y: auto;
		padding: 1rem;
		padding-top: max(1rem, env(safe-area-inset-top));
		background: var(--bg);
		color: var(--fg);
		user-select: text;
		-webkit-user-select: text;
	}

	.panel.wide {
		display: flex;
		flex-direction: column;
		width: min(48rem, 100%);
		/* 屏幕太矮（例如手机转过来）放不下时，中间缩到最小以后整个面板滚动。 */
		overflow-y: auto;
	}

	/* 标签很多时上面这块自己滚，最多占四成高度，给中间的缩略图留地方。 */
	.detail-top {
		flex: none;
		max-height: 40%;
		overflow-y: auto;
	}

	.detail-top :global(.picker) {
		margin-bottom: 0.6rem;
	}

	.detail-bottom {
		flex: none;
		margin-top: 0.5rem;
		padding-top: 0.75rem;
		border-top: 1px solid var(--border);
	}

	/* 宽屏上三块同时显示，没有分页标签，也不用网格版的相似作品。 */
	.detail-tabs,
	.detail-similar {
		display: none;
	}

	/* 手机上三块挤在一屏里每块都太小：改成三页，一次只显示一块，占满面板。 */
	@container reader (max-width: 599px) {
		.detail-tabs {
			display: flex;
			flex: none;
			gap: 0.25rem;
			margin-bottom: 0.75rem;
			padding: 0.2rem;
			border-radius: 8px;
			background: var(--sunken);
		}

		.detail-tabs button {
			flex: 1;
			padding: 0.4rem 0;
			border: none;
			border-radius: 6px;
			background: transparent;
			color: var(--muted);
			font: inherit;
			font-size: 0.9rem;
			cursor: pointer;
		}

		.detail-tabs button[aria-selected='true'] {
			background: var(--chip);
			color: var(--fg);
		}

		.detail-bottom {
			display: none;
		}

		.panel[data-tab='pages'] .detail-top,
		.panel[data-tab='pages'] .detail-similar,
		.panel[data-tab='info'] .overview-pages,
		.panel[data-tab='info'] .detail-similar,
		.panel[data-tab='similar'] .detail-top,
		.panel[data-tab='similar'] .overview-pages {
			display: none;
		}

		.panel[data-tab='info'] .detail-top,
		.panel[data-tab='similar'] .detail-similar {
			display: block;
			flex: 1 1 0;
			max-height: none;
			min-height: 0;
			overflow-y: auto;
		}

		.overview-pages {
			margin-top: 0;
		}
	}

	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
		margin-bottom: 1rem;
	}

	.actions a {
		padding: 0.4rem 0.8rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		color: var(--fg);
		font-size: 0.85rem;
		text-decoration: none;
	}

	.overview-pages {
		flex: 1 1 0;
		min-height: 5rem;
		margin-top: 0.75rem;
		overflow-y: auto;
	}

	.actions button {
		padding: 0.4rem 0.8rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: transparent;
		color: var(--fg);
		font: inherit;
		font-size: 0.85rem;
		cursor: pointer;
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
			height: 90%;
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
		border: 1px solid var(--border);
		border-radius: 8px;
		background: var(--sunken);
		color: var(--fg);
		font-size: 0.9rem;
		white-space: nowrap;
	}

	.next-volume a {
		padding: 0.3rem 0.8rem;
		border-radius: 6px;
		background: var(--accent);
		color: var(--on-accent);
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
		color: #fff;
		font-size: 0.85rem;
	}

	.empty {
		padding: 2rem;
		text-align: center;
	}
</style>
