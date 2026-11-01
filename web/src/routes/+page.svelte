<script lang="ts">
	import { goto } from '$app/navigation';
	import {
		ApiError,
		categories as listCategories,
		deleteCategory,
		random,
		suggestTags,
		type Category,
		type ArchiveItem,
		type Order,
		type Sort,
		type TagCount
	} from '#lib/api.ts';
	import { afterNavigate, invalidateAll, pushState, replaceState } from '$app/navigation';
	import { page as route } from '$app/state';
	import { serverListHref } from '#lib/nativeApp.ts';
	import BatchBar from '#lib/BatchBar.svelte';
	import BookDrawer from '#lib/BookDrawer.svelte';
	import BookmarkButton from '#lib/BookmarkButton.svelte';
	import { forgetMembers } from '#lib/membership.svelte.ts';
	import { savedTagStore } from '#lib/savedTags.svelte.ts';
	import Icon from '#lib/Icon.svelte';
	import { PAGE_SIZES, pageCount, type PageSize } from '#lib/pages.ts';
	import Pager from '#lib/Pager.svelte';
	import RecentRow from '#lib/RecentRow.svelte';
	import { settings } from '#lib/settings.svelte.ts';
	import TagLine from '#lib/TagLine.svelte';
	import TagPopover from '#lib/TagPopover.svelte';
	import { defaultOrder, filterLabel, libraryHref, SORTS, type UrlFilter } from '#lib/libraryUrl.ts';
	import { SvelteSet } from 'svelte/reactivity';
	import { onDestroy, untrack } from 'svelte';
	import { completeTerm, lastTerm, stripTrailingSeparator } from '#lib/query.ts';
	import { byline, formatDate, namespaceClass, rowTags } from '#lib/tags.ts';

	let { data } = $props();
	const prefs = settings();
	// 在桌面和 Android 客户端里，左上角的书库名打开客户端的服务器列表，用来切换服务器。
	let serverList = $derived(serverListHref(route.url.href));

	let items = $derived<ArchiveItem[]>(data.result?.items ?? []);
	let query = $derived(data.q);
	let pages = $derived(pageCount(data.result?.total ?? 0, data.pageSize));

	// 当前搜索条件的第几页，其余条件不变。
	function pageHref(page: number) {
		return libraryHref({
			q: data.q,
			sort: data.sort,
			order: data.order,
			category: data.category,
			filter: data.filter,
			tags: data.tags,
			page
		});
	}

	// 换排序方式时方向回到它的默认方向。换排序或方向时这台设备记住它，下次打开书库照样用。
	function navigate(q: string, sort: Sort, order: Order | null = sort === data.sort ? data.order : null) {
		if (sort !== data.sort || order !== data.order) {
			prefs.sort = sort;
			prefs.order = order;
			prefs.save();
		}
		goto(
			libraryHref({ q, sort, order, category: data.category, filter: data.filter, tags: data.tags })
		);
	}

	let order = $derived(data.order ?? defaultOrder(data.sort));

	function flipOrder() {
		navigate(data.q, data.sort, order === 'asc' ? 'desc' : 'asc');
	}

	// 批量模式下点卡片是选中它，不打开书；“全部搜索结果”勾上时改整个搜索结果。
	let selecting = $state(false);
	let selectAll = $state(false);
	const selected = new SvelteSet<number>();
	let batchMessage = $state<string | null>(null);

	// 换了搜索条件才清空选择；同一个条件重新加载（例如批量改完后的刷新）时保留。
	let listKey = $derived(
		JSON.stringify([data.q, data.tags, data.sort, data.order, data.category, data.filters])
	);
	$effect(() => {
		void listKey;
		untrack(() => {
			selected.clear();
			selectAll = false;
		});
	});

	function pickCard(event: MouseEvent) {
		if (!selecting) return;
		const card = (event.target as Element).closest<HTMLElement>('[data-id]');
		if (!card) return;
		event.preventDefault();
		event.stopPropagation();
		const id = Number(card.dataset.id);
		selectAll = false;
		if (selected.has(id)) selected.delete(id);
		else selected.add(id);
	}

	// 服务端改完标签约 10 秒后才重建快照，那时再取一次列表，筛选结果才对得上。
	const SNAPSHOT_DELAY = 11_000;
	let reloadTimer: ReturnType<typeof setTimeout> | undefined;
	onDestroy(() => {
		clearTimeout(reloadTimer);
		prefs.ungroupForBatch = false;
	});

	// 系列合并着时先等展开后的列表回来再进入批量模式，否则卡片和总数都还是合并的。
	let startingBatch = $state(false);
	async function startBatch() {
		if (startingBatch) return;
		if (data.group) {
			startingBatch = true;
			prefs.ungroupForBatch = true;
			try {
				await invalidateAll();
			} finally {
				startingBatch = false;
			}
			if (data.group) {
				prefs.ungroupForBatch = false;
				return;
			}
		}
		selecting = true;
	}

	function endBatch(message: string | null, tags = false) {
		selecting = false;
		selected.clear();
		selectAll = false;
		batchMessage = message;
		const regroup = prefs.ungroupForBatch;
		prefs.ungroupForBatch = false;
		if (!message) {
			if (regroup) invalidateAll();
			return;
		}
		// 批量改过的书，书签和分类勾选以重新加载的数据为准。
		forgetMembers();
		invalidateAll();
		// 只改了分类时不动已经排着的刷新，它可能是上一批改标签留下的。
		if (tags) {
			clearTimeout(reloadTimer);
			reloadTimer = setTimeout(invalidateAll, SNAPSHOT_DELAY);
		}
	}

	// 在当前的搜索、分类和筛选里随机打开一本。
	let picking = $state(false);
	let randomError = $state<string | null>(null);
	async function openRandom() {
		if (picking) return;
		picking = true;
		randomError = null;
		try {
			const id = await random({
				q: data.q,
				category: data.category,
				filters: data.filters,
				tags: data.tags
			});
			if (id === null) randomError = '没有符合条件的书，随机不到';
			else await goto(`/a/${id}/read`);
		} catch (error) {
			randomError = `随机失败：${error instanceof Error ? error.message : String(error)}`;
		} finally {
			picking = false;
		}
	}

	function submit(event: SubmitEvent) {
		event.preventDefault();
		closeSuggestions();
		// 补全会在末尾留一个逗号，提交时去掉，地址栏和状态行里不显示；转义的逗号属于词条，要保留。
		navigate(stripTrailingSeparator(query), data.sort);
	}

	// 搜索框补全：停止输入一小会儿后按最后一个词条查标签，新请求发出时取消旧请求。
	const SUGGEST_DELAY = 120;
	let suggestions = $state<TagCount[]>([]);
	let active = $state(-1);
	let suggestTimer: ReturnType<typeof setTimeout> | undefined;
	let suggestAbort: AbortController | undefined;

	function closeSuggestions() {
		clearTimeout(suggestTimer);
		suggestAbort?.abort();
		suggestions = [];
		active = -1;
	}

	function onInput() {
		closeSuggestions();
		const term = lastTerm(query);
		if (term.done || term.text === '') return;
		const abort = new AbortController();
		suggestAbort = abort;
		suggestTimer = setTimeout(async () => {
			try {
				suggestions = await suggestTags(term.text, abort.signal);
			} catch (error) {
				// 补全失败不影响搜索本身，被新输入取消的请求也走这里。
				if (!abort.signal.aborted) console.warn('标签补全失败', error);
			}
		}, SUGGEST_DELAY);
	}

	function pick(tag: string) {
		query = completeTerm(query, tag);
		closeSuggestions();
	}

	function onKeydown(event: KeyboardEvent) {
		// 输入法组字期间方向键和回车是给候选窗的，不能拦下来选标签。
		// Safari 在确认组字的那次 keydown 上 isComposing 为 false，只能靠 keyCode 229 认出。
		if (event.isComposing || event.keyCode === 229) return;
		if (suggestions.length === 0) return;
		if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
			event.preventDefault();
			// -1 表示没有选中，上下键在“没有选中”和各个候选之间循环。
			const slots = suggestions.length + 1;
			const step = event.key === 'ArrowDown' ? 1 : -1;
			active = ((active + 1 + step + slots) % slots) - 1;
		} else if (event.key === 'Enter' && active >= 0) {
			event.preventDefault();
			pick(suggestions[active].tag);
		} else if (event.key === 'Escape') {
			closeSuggestions();
		}
	}

	// 和 LRR 首页一样，搜索框下面是用户自己的分类，再点一次取消筛选。
	function categoryHref(id: number) {
		return libraryHref({
			q: data.q,
			sort: data.sort,
			order: data.order,
			category: data.category === id ? null : id,
			filter: data.filter,
			tags: data.tags
		});
	}

	// “新书”筛选，再点一次取消。
	function filterHref(filter: UrlFilter) {
		return libraryHref({
			q: data.q,
			sort: data.sort,
			order: data.order,
			category: data.category,
			filter: data.filter === filter ? null : filter,
			tags: data.tags
		});
	}

	// 收藏的标签可以选好几个，选中的都要有（交集），再点一次取消这一个。
	function tagHref(tag: string) {
		return libraryHref({
			q: data.q,
			sort: data.sort,
			order: data.order,
			category: data.category,
			filter: data.filter,
			tags: data.tags.includes(tag)
				? data.tags.filter((other) => other !== tag)
				: [...data.tags, tag]
		});
	}

	const pinnedTags = savedTagStore();

	// 筛选行的编辑状态：分类和收藏的标签旁边出现 ×，还能输入标签直接收藏。
	let editing = $state(false);
	let editError = $state<string | null>(null);
	let newTag = $state('');
	let pinChoices = $state<TagCount[]>([]);
	let pinRequest: AbortController | undefined;

	async function removeCategory(category: Category) {
		const ok = confirm(
			`删除分类“${category.name}”？分类里的 ${category.count} 本书不会删除，只是不再属于这个分类。`
		);
		if (!ok) return;
		editError = null;
		try {
			await deleteCategory(category.id);
			categoryList = await listCategories(fetch).catch(() =>
				categoryList.filter((other) => other.id !== category.id)
			);
			// 正在按这个分类筛选时回到不筛选。悬停时可能已经预加载了删除前的数据，
			// 换完地址再让 load 重新跑一遍，分类列表和结果都以删除后的为准。
			if (data.category === category.id) await goto(categoryHref(category.id));
			await invalidateAll();
		} catch (cause) {
			editError = `没有删除：${cause instanceof ApiError ? cause.message : String(cause)}`;
		}
	}

	async function suggestPinned() {
		pinRequest?.abort();
		const prefix = newTag.trim();
		if (!prefix) {
			pinChoices = [];
			return;
		}
		pinRequest = new AbortController();
		try {
			pinChoices = await suggestTags(prefix, pinRequest.signal, 8);
		} catch (cause) {
			if (!(cause instanceof DOMException && cause.name === 'AbortError')) pinChoices = [];
		}
	}

	function pinTag(tag: string) {
		if (!pinnedTags.has(tag)) pinnedTags.toggle(tag);
		newTag = '';
		pinChoices = [];
	}

	// 选中后又取消收藏的标签也照样显示，免得筛选着却看不到、取消不了。
	let tagButtons = $derived([
		...pinnedTags.tags,
		...data.tags.filter((tag) => !pinnedTags.has(tag))
	]);

	// 早先的阅读状态和未打标签筛选没有按钮了，旧链接打开时在结果说明里写出来。
	let hiddenFilter = $derived(
		data.filter && data.filter !== 'new' ? filterLabel(data.filter) : undefined
	);

	// 分类按钮和抽屉共用。抽屉里新建分类或改了成员后重新取一次带本数的列表，
	// 按钮跟着变；换一次搜索又回到这一页加载的数据。
	let categoryList = $derived(data.categories);

	// LRR 查重插件标记的疑似重复是待处理的问题，放在控制台里，不当书架显示。
	let shelves = $derived(categoryList.filter((category) => !category.duplicates));

	function toggleHideFinished() {
		prefs.hideFinished = !prefs.hideFinished;
		prefs.save();
		invalidateAll();
	}

	function splitTag(tag: string) {
		const colon = tag.indexOf(':');
		return colon < 0
			? { namespace: '', value: tag }
			: { namespace: tag.slice(0, colon), value: tag.slice(colon + 1) };
	}

	// 合并显示时，系列只占一张卡，点开是系列页。
	const seriesOf = (item: ArchiveItem) => (data.group ? item.series : null);

	function toggleSeries() {
		prefs.groupSeries = !prefs.groupSeries;
		prefs.save();
		invalidateAll();
	}

	// 每页本数变了以后原来的页码对不上，回到第一页。已经在第一页时地址不变，要强制重新加载。
	function changePageSize(size: PageSize) {
		prefs.pageSize = size;
		prefs.save();
		goto(pageHref(1), { invalidateAll: true });
	}

	function toggleView() {
		prefs.view = prefs.view === 'grid' ? 'list' : 'grid';
		prefs.save();
	}

	// 标签弹窗：鼠标悬停时打开，移开后稍等再关，让鼠标能移进弹窗里点标签；触屏点一下打开。
	const HOVER_DELAY = 150;
	let tagView = $state<{ item: ArchiveItem; anchor: DOMRect; sheet: boolean } | null>(null);
	let hoverTimer: ReturnType<typeof setTimeout> | undefined;

	function openTags(item: ArchiveItem, target: HTMLElement) {
		clearTimeout(hoverTimer);
		tagView = { item, anchor: target.getBoundingClientRect(), sheet: window.innerWidth < 600 };
	}

	function closeTags() {
		clearTimeout(hoverTimer);
		tagView = null;
	}

	function hoverIn(event: PointerEvent, item?: ArchiveItem) {
		if (event.pointerType !== 'mouse') return;
		clearTimeout(hoverTimer);
		const target = event.currentTarget as HTMLElement;
		if (item) hoverTimer = setTimeout(() => openTags(item, target), HOVER_DELAY);
	}

	function hoverOut(event: PointerEvent) {
		if (event.pointerType !== 'mouse') return;
		clearTimeout(hoverTimer);
		hoverTimer = setTimeout(() => (tagView = null), HOVER_DELAY);
	}

	// 点标题或标签行打开详情抽屉；鼠标悬停在标签行上仍然弹出标签浮层。
	let drawer = $state.raw<ArchiveItem | null>(null);
	// 抽屉里的标签链接会换一个搜索，留在原页面上的抽屉会挡住新结果；后退回到抽屉开着的
	// 那条记录时再找回那本书。
	afterNavigate(() => {
		const id = route.state.drawer;
		drawerInHistory = id;
		drawer = id === undefined ? null : findBook(id);
	});

	// 抽屉里的书可能来自列表，也可能来自最近阅读；前进后退回到抽屉开着的记录时按 ID 找回来。
	function findBook(id: number): ArchiveItem | null {
		return items.find((item) => item.id === id) ?? data.recent.find((item) => item.id === id) ?? null;
	}

	// 打开抽屉时加一条历史记录，手机的返回手势和浏览器的后退只关抽屉，不离开书库。
	function openDrawer(item: ArchiveItem) {
		closeTags();
		if (drawer === null) pushState('', { drawer: item.id });
		else replaceState('', { drawer: item.id });
		drawer = item;
	}

	function closeDrawer() {
		if (drawer === null) return;
		drawer = null;
		if (route.state.drawer !== undefined) history.back();
	}

	// 抽屉跟着历史记录走：后退弹掉那条记录时关上，前进回来时在当前列表里找回那本再打开。
	let drawerInHistory: number | undefined = undefined;
	$effect(() => {
		const id = route.state.drawer;
		if (id === drawerInHistory) return;
		drawerInHistory = id;
		untrack(() => {
			drawer = id === undefined ? null : findBook(id);
		});
	});
</script>

<header>
	{#if serverList}
		<a class="brand" href={serverList} title="切换服务器">{data.libraryName}</a>
	{:else}
		<a class="brand" href="/" onclick={() => (query = '')}>{data.libraryName}</a>
	{/if}
	<form onsubmit={submit} role="search">
		<Icon name="search" size={16} />
		<input
			type="search"
			bind:value={query}
			oninput={onInput}
			onkeydown={onKeydown}
			onblur={closeSuggestions}
			placeholder="标题或标签，如 artist:名字，-排除"
			enterkeyhint="search"
			autocapitalize="off"
			autocomplete="off"
			spellcheck="false"
			role="combobox"
			aria-autocomplete="list"
			aria-controls="tag-suggestions"
			aria-expanded={suggestions.length > 0}
			aria-activedescendant={active >= 0 ? `suggestion-${active}` : undefined}
		/>
		{#if suggestions.length > 0}
			<ul id="tag-suggestions" class="suggestions" role="listbox">
				{#each suggestions as suggestion, index (suggestion.tag)}
					{@const tag = splitTag(suggestion.tag)}
					<!-- 按下时阻止默认动作，输入框不失焦，列表不会先关掉；松开后的 click 才选中。
					     原来在 pointerdown 里就选中并关掉列表，手机上随后的 click 会落到列表下面的元素上。 -->
					<!-- 键盘在输入框里用上下键和回车选，见 onKeydown。 -->
					<!-- svelte-ignore a11y_click_events_have_key_events -->
					<li
						id="suggestion-{index}"
						role="option"
						aria-selected={index === active}
						class:active={index === active}
						onpointerdown={(event) => event.preventDefault()}
						onmousedown={(event) => event.preventDefault()}
						onclick={() => pick(suggestion.tag)}
					>
						<span>
							{#if tag.namespace}<span class="ns {namespaceClass(tag.namespace)}"
									>{tag.namespace}:</span
								>{/if}{tag.value}
						</span>
						<span class="count">{suggestion.count}</span>
					</li>
				{/each}
			</ul>
		{/if}
	</form>
	<div class="controls">
		<select
			value={data.sort}
			onchange={(event) => navigate(data.q, event.currentTarget.value as Sort)}
			aria-label="排序"
		>
			{#each SORTS as [value, label] (value)}
				<option {value}>{label}</option>
			{/each}
			{#if !SORTS.some(([value]) => value === data.sort)}
				<option value={data.sort}>{data.sort}</option>
			{/if}
		</select>
		<button
			class="icon-button"
			onclick={flipOrder}
			aria-label={order === 'asc' ? '当前从小到大，改成从大到小' : '当前从大到小，改成从小到大'}
			title={order === 'asc' ? '从小到大' : '从大到小'}
		>
			<Icon name={order === 'asc' ? 'arrow-up' : 'arrow-down'} size={18} />
		</button>
		<button
			class="icon-button"
			onclick={openRandom}
			disabled={picking}
			aria-label="随机一本"
			title="随机一本"
		>
			<Icon name="shuffle" size={18} />
		</button>
		<button
			class="icon-button"
			class:on={selecting}
			onclick={() => (selecting ? endBatch(null) : startBatch())}
			disabled={startingBatch}
			aria-pressed={selecting}
			aria-label="批量操作"
			title="批量操作"
		>
			<Icon name="check-square" size={18} />
		</button>
		<button
			class="icon-button"
			onclick={toggleView}
			aria-label={prefs.view === 'grid' ? '列表视图' : '封面视图'}
		>
			<Icon name={prefs.view === 'grid' ? 'list' : 'grid'} size={18} />
		</button>
		<a class="icon-button" href="/admin" aria-label="控制台" title="控制台">
			<Icon name="settings" size={18} />
		</a>
	</div>
</header>

<div class="page">

{#if data.queryError}
	<p class="status error">{data.queryError}</p>
{:else if data.result && (hiddenFilter || data.result.degraded || (!selecting && items.some((item) => item.series)))}
	<p class="status">
		{#if hiddenFilter}<span>筛选：{hiddenFilter} ·</span>{/if}
		{#if data.result.degraded}<span>部分通配符还不支持，结果可能偏多 ·</span>{/if}
		<!-- 批量操作时始终展开各卷，不能切回合并。 -->
		{#if !selecting && items.some((item) => item.series)}
			<button class="link" onclick={toggleSeries}
				>{data.group ? '系列已合并，展开各卷' : '按系列合并'}</button
			>
		{/if}
	</p>
{/if}
{#if randomError}
	<p class="status error" role="alert">{randomError}</p>
{/if}
{#if editError || pinnedTags.error}
	<p class="status error" role="alert">{editError ?? `收藏标签没有保存：${pinnedTags.error}`}</p>
{/if}
<!-- 和 LRR 一样一排：新书、各个分类，按钮样式相同，只靠图标区分；最右边是开关和本数。 -->
<nav class="facets" aria-label="筛选和分类">
	<a href={filterHref('new')} aria-current={data.filter === 'new' ? 'true' : undefined}>
		<span class="dot" aria-hidden="true"></span>
		<span>新书</span>
		{#if data.newCount !== null}<span class="count">{data.newCount}</span>{/if}
	</a>
	{#each shelves as category (category.id)}
		<span class="item">
			<a
				href={categoryHref(category.id)}
				aria-current={data.category === category.id ? 'true' : undefined}
				title={category.dynamic ? '动态分类' : undefined}
			>
				{#if category.bookmark}<Icon name="bookmark" size={13} filled />{/if}
				<span>{category.name}</span>
				<span class="count">{category.count}</span>
			</a>
			{#if editing}
				<button class="remove" aria-label="删除分类 {category.name}" onclick={() => removeCategory(category)}
					>×</button
				>
			{/if}
		</span>
	{/each}
	{#each tagButtons as tag (tag)}
		<span class="item">
			<a
				class={namespaceClass(splitTag(tag).namespace)}
				href={tagHref(tag)}
				aria-current={data.tags.includes(tag) ? 'true' : undefined}
				title="收藏的标签，可以同时选几个"
			>
				<Icon name="tag" size={13} />
				<span>{tag}</span>
			</a>
			{#if editing && pinnedTags.has(tag)}
				<button
					class="remove"
					aria-label="取消收藏标签 {tag}"
					disabled={pinnedTags.isPending(tag)}
					onclick={() => pinnedTags.toggle(tag)}>×</button
				>
			{/if}
		</span>
	{/each}
	<span class="end">
		<button aria-pressed={editing} onclick={() => (editing = !editing)} title="删除分类、管理收藏的标签">
			<Icon name="edit" size={13} />编辑
		</button>
		<button aria-pressed={prefs.hideFinished} onclick={toggleHideFinished}>隐藏读完的</button>
		{#if data.result}<span class="total">共 {data.result.total} 本</span>{/if}
	</span>
</nav>
{#if editing}
	<!-- 输入时提示书库里已有的标签，选中就收藏，不用先打开一本书。 -->
	<form class="add-tag" onsubmit={(event) => event.preventDefault()}>
		<input
			bind:value={newTag}
			oninput={suggestPinned}
			placeholder="收藏标签…"
			aria-label="收藏标签"
			autocapitalize="off"
			autocomplete="off"
			spellcheck="false"
		/>
		{#if pinChoices.length > 0}
			<ul class="suggestions">
				{#each pinChoices as choice (choice.tag)}
					<li>
						<button type="button" onclick={() => pinTag(choice.tag)}>
							<span class={namespaceClass(splitTag(choice.tag).namespace)}>{choice.tag}</span>
							<span class="count">{choice.count}</span>
						</button>
					</li>
				{/each}
			</ul>
		{/if}
	</form>
{/if}

{#if data.recent.length > 0}
	<RecentRow items={data.recent} ondetail={openDrawer} />
{/if}

{#if batchMessage}
	<p class="status" role="status">{batchMessage}</p>
{/if}

<Pager page={data.page} {pages} href={pageHref} />

{#if prefs.view === 'grid'}
	<main class="grid" onclickcapture={pickCard}>
		{#each items as item (item.id)}
			{@const series = seriesOf(item)}
			<div class="card" data-id={item.id} class:picked={selecting && (selectAll || selected.has(item.id))}>
				{#if series}
					<a class="title" href="/s/{series.id}" title={series.name}>{series.name}</a>
				{:else}
					<button class="title" onclick={() => openDrawer(item)} title={item.title}>{item.title}</button>
				{/if}
				<a
					class="cover"
					class:stack={series}
					class:selected={drawer?.id === item.id}
					href={series ? `/s/${series.id}` : `/a/${item.id}/read`}>
					<img src={item.cover_url} alt="" loading="lazy" />
					{#if item.is_new && !item.progress}<span class="badge new">新</span>{/if}
					<span class="badge pages">{series ? `${series.volumes} 卷` : `${item.page_count}P`}</span>
					{#if !series && item.progress && item.page_count > 0}
						<div
							class="progress"
							style:width="{((item.progress.page + 1) / item.page_count) * 100}%"
						></div>
					{/if}
				</a>
				{#if item.tags.length > 0}
					<button
						class="tags"
						aria-label="查看全部标签"
						aria-expanded={tagView?.item.id === item.id}
						onclick={() => openDrawer(item)}
						onpointerenter={(event) => hoverIn(event, item)}
						onpointerleave={hoverOut}
					>
						<TagLine groups={item.tags} />
					</button>
				{/if}
			</div>
		{/each}
	</main>
{:else}
	<main class="list" onclickcapture={pickCard}>
		<!-- 电脑上是一张表：封面、标题和作者、几个标签、页数、加入时间、进度、书签。 -->
		<div class="list-head" aria-hidden="true">
			<span></span><span>标题</span><span>标签</span><span class="num">页数</span><span
				>加入时间</span
			><span>进度</span><span></span>
		</div>
		{#each items as item (item.id)}
			{@const series = seriesOf(item)}
			{@const href = series ? `/s/${series.id}` : `/a/${item.id}/read`}
			{@const read = item.progress ? item.progress.page + 1 : 0}
			<article class="row" data-id={item.id} class:picked={selecting && (selectAll || selected.has(item.id))}>
				<a class="thumb" {href} aria-label="阅读 {series?.name ?? item.title}">
					<img src={item.cover_url} alt="" loading="lazy" />
				</a>
				<div class="main">
					{#if series}
						<a class="title" {href}>{series.name}</a>
					{:else}
						<button class="title" onclick={() => openDrawer(item)}>{item.title}</button>
					{/if}
					{#if byline(item.tags)}<span class="byline">{byline(item.tags)}</span>{/if}
				</div>
				<div class="chips">
					{#each rowTags(item.tags, 4) as tag, index (index)}
						<span class="chip {namespaceClass(tag.namespace)}">{tag.text}</span>
					{/each}
				</div>
				<span class="pages">{series ? `${series.volumes} 卷` : `${item.page_count} 页`}</span>
				<span class="added">{formatDate(item.date_added)}</span>
				<span class="reading">
					{#if !series}
						<span class="track"
							><span
								style:width="{item.page_count ? (read / item.page_count) * 100 : 0}%"
							></span></span
						>
						{#if read}{read} / {item.page_count}{:else}<span class:fresh={item.is_new}
								>{item.is_new ? '新' : '没读过'}</span
							>{/if}
					{/if}
				</span>
				<span class="mark">
					{#if !series}
						<BookmarkButton
							archive={item.id}
							members={item.categories}
							bind:categories={categoryList}
							bar
						/>
					{/if}
				</span>
			</article>
		{/each}
	</main>
{/if}
</div>

{#if drawer}
	<BookDrawer item={drawer} bind:categories={categoryList} onclose={closeDrawer} />
{/if}

{#if tagView}
	<TagPopover
		item={tagView.item}
		anchor={tagView.anchor}
		sheet={tagView.sheet}
		onclose={closeTags}
		onpointerenter={(event) => hoverIn(event)}
		onpointerleave={hoverOut}
	/>
{/if}

<svelte:window
	onkeydown={(event) => {
		if (event.key !== 'Escape') return;
		closeTags();
		closeDrawer();
	}}
	onscroll={() => tagView && !tagView.sheet && closeTags()}
	onpointerdown={(event) => {
		// 点在浮层和标签行以外的地方时关闭；底部面板由遮罩处理。
		const target = event.target as Element;
		if (tagView && !tagView.sheet && !target.closest('.popover, .tags')) closeTags();
	}}
/>

<Pager page={data.page} {pages} href={pageHref} />
{#if data.result && data.result.total > PAGE_SIZES[0]}
	<p class="page-size">
		<label>
			每页数量
			<select
				value={data.pageSize}
				onchange={(event) => changePageSize(Number(event.currentTarget.value) as PageSize)}
			>
				{#each PAGE_SIZES as size (size)}
					<option value={size}>{size} 本</option>
				{/each}
			</select>
		</label>
	</p>
{/if}
{#if items.length === 0 && data.result && data.page > 1}
	<p class="footer">这一页没有书了，<a href={pageHref(1)}>回到第一页</a></p>
{/if}

{#if selecting}
	<!-- 底部操作栏盖住的高度，让最后一排书也能点到。 -->
	<div class="bar-space"></div>
	<BatchBar
		selection={selectAll
			? { search: { q: data.q, category: data.category, filters: data.filters, tags: data.tags } }
			: { ids: [...selected] }}
		count={selectAll ? (data.result?.total ?? 0) : selected.size}
		total={data.result?.total ?? 0}
		bind:all={selectAll}
		categories={categoryList}
		ondone={endBatch}
		oncancel={() => endBatch(null)}
	/>
{/if}

<style>
	header {
		position: sticky;
		top: 0;
		z-index: 2;
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.5rem 0.75rem;
		padding: 0.6rem 1rem;
		padding-top: max(0.6rem, env(safe-area-inset-top));
		background: color-mix(in srgb, var(--bg) 92%, transparent);
		backdrop-filter: blur(8px);
		-webkit-backdrop-filter: blur(8px);
		border-bottom: 1px solid var(--border);
	}

	/* 电脑上书库内容居中，宽度是窗口的 90%，和 LRR 默认的 Modern 主题一样不设上限；
	   顶栏的背景占满宽，里面的内容和下面对齐。 */
	@media (min-width: 1024px) {
		.page {
			max-width: 90%;
			margin: 0 auto;
		}

		header {
			padding-inline: calc(5% + 1.25rem);
		}
	}

	.brand {
		font-weight: 700;
		font-size: 1.1rem;
		text-decoration: none;
	}

	form {
		position: relative;
		flex: 1;
		display: flex;
		align-items: center;
		gap: 0.4rem;
		min-width: 0;
		max-width: 44rem;
		height: 2.25rem;
		padding: 0 0.7rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: var(--surface);
		color: var(--muted);
	}

	input {
		flex: 1;
		min-width: 0;
		border: none;
		outline: none;
		background: transparent;
		color: var(--fg);
	}

	.controls {
		display: flex;
		gap: 0.5rem;
		margin-left: auto;
	}

	select {
		height: 2.25rem;
		padding: 0 0.5rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: var(--surface);
	}

	/* 手机上搜索框单独占一行，品牌和按钮在上面一行。 */
	@media (max-width: 599px) {
		form {
			order: 3;
			flex-basis: 100%;
			max-width: none;
		}
	}

	.suggestions {
		position: absolute;
		top: calc(100% + 4px);
		left: 0;
		right: 0;
		z-index: 3;
		margin: 0;
		padding: 0.25rem 0;
		list-style: none;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: var(--raised);
		box-shadow: 0 6px 18px rgb(0 0 0 / 0.25);
		color: var(--fg);
	}

	.suggestions li {
		display: flex;
		justify-content: space-between;
		gap: 0.75rem;
		padding: 0.35rem 0.7rem;
		cursor: pointer;
		overflow-wrap: anywhere;
	}

	.suggestions li.active,
	.suggestions li:hover {
		background: var(--chip);
	}

	.suggestions .ns:not([class*='ns-']) {
		color: var(--muted);
	}

	.count {
		flex: none;
		color: var(--faint);
		font-size: 0.8rem;
		font-variant-numeric: tabular-nums;
	}

	/* 放不下时换行。手机上原来是一排横向滑动，最右边的“编辑”和“隐藏读完的”要滑到头才看得见。 */
	.facets {
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem 0.35rem;
		margin: 0.5rem 0 0;
		padding: 0 1rem 0.2rem;
	}

	.facets a,
	.facets button {
		flex: none;
		display: inline-flex;
		align-items: center;
		gap: 0.35rem;
		padding: 0.15rem 0.55rem;
		border: 1px solid var(--chip-border);
		border-radius: 5px;
		background: var(--chip);
		color: inherit;
		font: inherit;
		font-size: 0.8rem;
		text-decoration: none;
		cursor: pointer;
	}

	.facets a:hover,
	.facets button:hover {
		color: var(--link);
	}

	.facets a[aria-current='true'],
	.facets button[aria-pressed='true'] {
		background: var(--fg);
		border-color: var(--fg);
		color: var(--bg);
	}

	.facets a[aria-current='true'] .count {
		color: inherit;
		opacity: 0.7;
	}

	/* 编辑时分类和收藏的标签后面跟一个 ×。 */
	.facets .item {
		display: inline-flex;
		flex: 0 1 auto;
		align-items: center;
		gap: 0.15rem;
		min-width: 0;
		max-width: 100%;
	}

	/* 很长的分类名或标签不撑出屏幕：按钮最宽到一行，名字截断成省略号。 */
	.facets .item a {
		flex: 0 1 auto;
		min-width: 0;
	}

	.facets .item a > span:not(.count) {
		overflow: hidden;
		white-space: nowrap;
		text-overflow: ellipsis;
	}

	.facets .remove {
		padding: 0 0.35rem;
		border-color: transparent;
		background: transparent;
		color: var(--muted);
		font-size: 0.95rem;
		line-height: 1;
	}

	.facets .remove:hover {
		color: var(--danger);
	}

	/* 顶栏搜索框的 form 样式（边框、底色、固定高度）在这里不要。 */
	.add-tag {
		position: relative;
		display: block;
		width: min(20rem, calc(100% - 2rem));
		height: auto;
		margin: 0.4rem 1rem 0;
		padding: 0;
		border: none;
		background: none;
	}

	.add-tag input {
		width: 100%;
		height: 2rem;
		padding: 0 0.6rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: var(--surface);
		color: var(--fg);
	}

	.add-tag .suggestions button {
		display: flex;
		justify-content: space-between;
		gap: 0.75rem;
		width: 100%;
		padding: 0.35rem 0.7rem;
		border: none;
		background: none;
		color: inherit;
		font: inherit;
		text-align: left;
		cursor: pointer;
	}

	.add-tag .suggestions button:hover {
		background: var(--surface);
	}

	@media (min-width: 1024px) {
		.add-tag {
			margin: 0.4rem 1.25rem 0;
		}
	}

	/* “新书”的图标，和卡片上“新”角标同色。 */
	.facets .dot {
		width: 0.5rem;
		height: 0.5rem;
		border-radius: 50%;
		background: var(--new);
	}

	.facets .end {
		display: inline-flex;
		flex: none;
		align-items: center;
		gap: 0.6rem;
		margin-left: auto;
		padding-left: 0.6rem;
	}

	.facets .total {
		color: var(--muted);
		font-size: 0.8rem;
		white-space: nowrap;
	}

	@media (min-width: 1024px) {
		.facets {
			margin: 0.5rem 0 0;
			padding: 0 1.25rem 0.2rem;
		}
	}

	.status {
		margin: 0.6rem 1rem 0;
		font-size: 0.85rem;
		color: var(--muted);
	}

	.status .link {
		padding: 0;
		border: 0;
		background: none;
		color: var(--link);
		font: inherit;
		cursor: pointer;
	}

	.grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(155px, 1fr));
		gap: 0.6rem;
		padding: 0.75rem;
	}

	/* 内容宽度是窗口的 90%（见 .page），一排的本数随窗口变：1280 宽 5 本，1920 宽 7 本，2560 宽 10 本。 */
	@media (min-width: 1024px) {
		.grid {
			grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
			gap: 1rem;
			padding: 1.25rem;
		}
	}

	.card {
		display: flex;
		flex-direction: column;
		gap: 0.4rem;
		min-width: 0;
		padding: 0.45rem 0.45rem 0.55rem;
		border-radius: 9px;
		background: var(--surface);
		text-decoration: none;
	}

	/* 标题是按钮（打开详情抽屉），去掉按钮自带的样式，看起来和原来的链接一样。 */
	button.title {
		padding: 0;
		border: none;
		background: none;
		color: inherit;
		font: inherit;
		cursor: pointer;
	}

	button.title:hover {
		color: var(--link);
	}

	.card .title {
		text-decoration: none;
		min-height: 2.5em;
		font-size: 0.85rem;
		font-weight: 600;
		line-height: 1.25;
		text-align: center;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}

	.cover {
		display: block;
		position: relative;
		aspect-ratio: 5 / 7;
		overflow: hidden;
		border-radius: 6px;
		background: var(--chip);
		outline: 2px solid transparent;
		outline-offset: 2px;
		transition: outline-color 200ms ease;
	}

	/* 详情抽屉里正在看的那本。 */
	.cover.selected {
		outline-color: var(--accent);
	}

	.tags {
		-webkit-tap-highlight-color: transparent;
		padding: 0;
		border: none;
		background: none;
		text-align: left;
		cursor: pointer;
	}

	.tags:hover :global(.tagline) {
		color: var(--fg);
	}

	.cover img {
		display: block;
		width: 100%;
		height: 100%;
		object-fit: cover;
		object-position: center top;
	}

	/* 系列卡的封面后面错开两层，看起来是一叠书。 */
	.cover.stack {
		overflow: visible;
		margin: 6px 6px 0 0;
		box-shadow:
			3px -3px 0 0 var(--surface),
			3px -3px 0 1px var(--border),
			6px -6px 0 0 var(--surface),
			6px -6px 0 1px var(--border);
	}

	.cover.stack img {
		border-radius: 6px;
	}

	.badge {
		position: absolute;
		padding: 0.05rem 0.35rem;
		border-radius: 4px;
		font-size: 0.72rem;
		line-height: 1.4;
	}

	.badge.new {
		top: 0.3rem;
		left: 0.3rem;
		background: var(--new);
		color: var(--on-accent);
	}

	.badge.pages {
		right: 0.3rem;
		bottom: 0.45rem;
		background: rgb(0 0 0 / 0.6);
		color: #eee;
	}

	.progress {
		position: absolute;
		left: 0;
		bottom: 0;
		height: 4px;
		background: var(--accent);
	}

	.list {
		display: flex;
		flex-direction: column;
		margin: 0.75rem;
		border: 1px solid var(--surface);
		border-radius: 10px;
		overflow: hidden;
	}

	/* 手机上每行三块：封面、标题和标签、书签；页数、日期和进度挤在标签下面一行。 */
	.row {
		display: grid;
		grid-template-columns: 48px minmax(0, 1fr) auto auto 2.5rem;
		grid-template-areas:
			'thumb main main main mark'
			'thumb chips chips chips mark'
			'thumb pages added reading mark';
		gap: 0.25rem 0.6rem;
		align-items: center;
		min-width: 0;
		padding: 0.5rem 0.6rem;
		border-top: 1px solid var(--surface);
	}

	.list-head {
		display: none;
	}

	/* 电脑上是一张表，和设计画布的列表视图一致。 */
	@media (min-width: 900px) {
		.list-head,
		.row {
			display: grid;
			grid-template-columns: 56px minmax(0, 3fr) minmax(0, 2fr) 4.5rem 6.5rem 8rem 2.5rem;
			grid-template-areas: 'thumb main chips pages added reading mark';
			gap: 0.75rem;
			align-items: center;
			padding: 0.5rem 0.75rem;
		}

		.list-head {
			background: var(--sunken);
			color: var(--muted);
			font-size: 0.75rem;
		}

		.list-head .num,
		.pages {
			text-align: right;
		}
	}

	.list > .row:first-of-type {
		border-top: none;
	}

	.thumb {
		grid-area: thumb;
		align-self: start;
	}

	.thumb img {
		display: block;
		width: 44px;
		aspect-ratio: 5 / 7;
		object-fit: cover;
		object-position: center top;
		border-radius: 4px;
		background: var(--chip);
	}

	.main {
		grid-area: main;
		display: flex;
		flex-direction: column;
		gap: 0.15rem;
		min-width: 0;
	}

	.main .title {
		overflow: hidden;
		font-weight: 600;
		line-height: 1.35;
		text-align: left;
		text-decoration: none;
		white-space: nowrap;
		text-overflow: ellipsis;
	}

	.byline {
		overflow: hidden;
		color: var(--artist);
		font-size: 0.75rem;
		white-space: nowrap;
		text-overflow: ellipsis;
	}

	.chips {
		grid-area: chips;
		display: flex;
		gap: 0.25rem;
		min-width: 0;
		overflow: hidden;
	}

	.chips .chip {
		flex: none;
		padding: 0 0.35rem;
		border: 1px solid var(--chip-border);
		border-radius: 5px;
		background: var(--chip);
		font-size: 0.7rem;
		line-height: 1.2rem;
	}

	.pages,
	.added,
	.reading {
		color: var(--muted);
		font-size: 0.75rem;
		font-variant-numeric: tabular-nums;
		white-space: nowrap;
	}

	.pages {
		grid-area: pages;
	}

	.added {
		grid-area: added;
	}

	.reading {
		grid-area: reading;
		display: flex;
		flex-direction: column;
		gap: 0.2rem;
		min-width: 4rem;
	}

	.track {
		display: block;
		height: 4px;
		border-radius: 2px;
		background: var(--border);
	}

	.track span {
		display: block;
		height: 100%;
		border-radius: 2px;
		background: var(--accent);
	}

	.mark {
		grid-area: mark;
		justify-self: end;
	}

	.fresh {
		color: var(--new);
	}

	.bar-space {
		height: 8rem;
	}

	.picked {
		outline: 2px solid var(--accent);
		outline-offset: -2px;
	}

	.page-size {
		margin: 0.75rem 1rem;
		text-align: center;
		color: var(--muted);
		font-size: 0.85rem;
	}

	.page-size select {
		height: 2rem;
		margin-left: 0.4rem;
	}

	.footer {
		min-height: 3rem;
		padding: 1rem;
		text-align: center;
		color: var(--muted);
		font-size: 0.85rem;
	}
</style>
