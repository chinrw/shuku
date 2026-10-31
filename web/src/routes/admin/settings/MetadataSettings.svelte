<script lang="ts">
	// 控制台设置页的“元数据”卡片：文件名、ComicInfo、E-Hentai 和中文翻译、Bangumi、AniList 的选项，
	// 以及每个在线来源的队列和候选。cookie 和令牌只写不读，输入框留空表示不改。
	import { onMount } from 'svelte';
	import {
		clearMetadataQueue,
		metadataSettings,
		metadataStatus,
		resumeMetadataQueue,
		saveMetadataSettings,
		type CookieName,
		type MetadataSettings,
		type MetadataStatus,
		type OnlineProvider
	} from '#lib/admin.ts';
	import { ApiError } from '#lib/api.ts';

	/** 队列在跑时多久刷新一次状态。 */
	const STATUS_INTERVAL_MS = 5000;

	let form = $state<MetadataSettings | null>(null);
	let status = $state<MetadataStatus | null>(null);
	let cookies = $state<Record<CookieName, string>>({
		ipb_member_id: '',
		ipb_pass_hash: '',
		star: '',
		igneous: ''
	});
	let token = $state('');
	let saving = $state(false);
	/** 最近一次刷新状态失败的原因；成功后清空。失败时仍显示上一次的状态，标明已过期。 */
	let statusError = $state('');
	let message = $state('');
	let error = $state('');

	const cookieFields: [CookieName, string][] = [
		['ipb_member_id', 'ipb_member_id'],
		['ipb_pass_hash', 'ipb_pass_hash'],
		['star', 'star（可选）'],
		['igneous', 'igneous（可选，ExHentai 用）']
	];

	const providers: [OnlineProvider, string][] = [
		['ehentai', 'E-Hentai'],
		['bangumi', 'Bangumi'],
		['anilist', 'AniList']
	];

	function describe(cause: unknown) {
		return cause instanceof ApiError ? cause.message : '网络错误';
	}

	async function refreshStatus() {
		try {
			status = await metadataStatus();
			statusError = '';
		} catch (cause) {
			statusError = describe(cause);
		}
	}

	onMount(() => {
		metadataSettings()
			.then((settings) => (form = settings))
			.catch((cause) => (error = `读取设置失败：${describe(cause)}`));
		refreshStatus();
		const timer = setInterval(refreshStatus, STATUS_INTERVAL_MS);
		return () => clearInterval(timer);
	});

	/** `clear` 是要清除的 cookie，或者 `'token'` 清除 Bangumi 的令牌。 */
	async function save(clear?: CookieName | 'token') {
		if (!form || saving) return;
		saving = true;
		message = '';
		error = '';
		const { cookies_set: _, ...ehentai } = form.ehentai;
		const { token_set: __, ...bangumi } = form.bangumi;
		const typed = Object.fromEntries(
			Object.entries(cookies).filter(([, value]) => value.trim() !== '')
		) as Partial<Record<CookieName, string>>;
		if (clear && clear !== 'token') typed[clear] = '';
		const newToken = clear === 'token' ? '' : token.trim() || undefined;
		try {
			form = await saveMetadataSettings({
				replace_title: form.replace_title,
				filename: form.filename,
				comicinfo: form.comicinfo,
				ehentai: { ...ehentai, cookies: typed },
				translation: form.translation,
				bangumi: newToken === undefined ? bangumi : { ...bangumi, token: newToken },
				anilist: form.anilist
			});
			cookies = { ipb_member_id: '', ipb_pass_hash: '', star: '', igneous: '' };
			token = '';
			message = '已保存';
			await refreshStatus();
		} catch (cause) {
			error = `没有保存：${describe(cause)}`;
		} finally {
			saving = false;
		}
	}

	async function resume(provider: OnlineProvider) {
		await resumeMetadataQueue(provider).catch((cause) => (error = describe(cause)));
		await refreshStatus();
	}

	async function clearQueue(provider: OnlineProvider, label: string) {
		if (!confirm(`清空 ${label} 的队列？队列里的书不会再去查。`)) return;
		await clearMetadataQueue(provider).catch((cause) => (error = describe(cause)));
		await refreshStatus();
	}
</script>

<section class="adm-card">
	<h2>元数据</h2>
	<p class="adm-note">
		代替 LRR 的文件名解析、ComicInfo、E-Hentai 和 ETagCN 插件，另有给按卷出版的漫画用的 Bangumi 和
		AniList。只给扫描新加入的书自动运行，从 LRR 迁移来的书不动；详情页的“获取元数据”可以对单本手动运行。
		和 LRR 一样只加标签，不删已有的；来源和简介只在书还没有时写入。同人志和普通漫画各跑一个实例，这里的开关各配各的。
	</p>
	{#if form}
		<label class="check"><input type="checkbox" bind:checked={form.replace_title} />用找到的标题替换原标题</label>

		<h3>文件名</h3>
		<label class="check"><input type="checkbox" bind:checked={form.filename.auto} />新书自动运行</label>
		<label class="check"
			><input type="checkbox" bind:checked={form.filename.trailing_tags} />末尾的 {'{a, b}'} 当作标签</label
		>
		<label class="check"
			><input type="checkbox" bind:checked={form.filename.keep_all_captures} />其他括号里的内容放进 parsed:
			命名空间</label
		>
		<label class="field">
			<span class="adm-label">正则（Rust 语法，命名分组决定命名空间；留空用默认的同人志格式）</span>
			<input class="adm-mono" bind:value={form.filename.regex} placeholder="默认" spellcheck="false" />
		</label>

		<h3>ComicInfo.xml</h3>
		<label class="check"><input type="checkbox" bind:checked={form.comicinfo.auto} />新书自动运行</label>

		<h3>E-Hentai</h3>
		<label class="check"
			><input type="checkbox" bind:checked={form.ehentai.auto} />新书放进后台队列，每本之间隔 4 秒</label
		>
		<label class="check"
			><input type="checkbox" bind:checked={form.ehentai.review} />查到的先放进候选，确认后才写入</label
		>
		<label class="check"><input type="checkbox" bind:checked={form.ehentai.exhentai} />在 ExHentai 上搜索</label>
		<label class="check"
			><input type="checkbox" bind:checked={form.ehentai.use_thumbnail} />先用封面图搜索</label
		>
		<label class="check"
			><input type="checkbox" bind:checked={form.ehentai.search_gid} />标题里有 [画廊 ID] 时先按 ID 搜索</label
		>
		<label class="check"
			><input type="checkbox" bind:checked={form.ehentai.japanese_title} />有日文标题时用日文标题</label
		>
		<label class="check"
			><input type="checkbox" bind:checked={form.ehentai.extra_tags} />另加上传者和发布时间标签</label
		>
		<label class="check"><input type="checkbox" bind:checked={form.ehentai.expunged} />也搜索已删除的画廊</label>
		<label class="field">
			<span class="adm-label">搜索限定语言（例如 chinese，留空不限）</span>
			<input bind:value={form.ehentai.language} spellcheck="false" />
		</label>
		<div class="cookies">
			{#each cookieFields as [name, label] (name)}
				<label class="field">
					<span class="adm-label"
						>{label} · {form.ehentai.cookies_set[name] ? '已设置，留空不改' : '未设置'}</span
					>
					<span class="cookie">
						<input type="password" autocomplete="off" bind:value={cookies[name]} />
						{#if form.ehentai.cookies_set[name]}
							<button class="adm-button" disabled={saving} onclick={() => save(name)}>清除</button>
						{/if}
					</span>
				</label>
			{/each}
		</div>

		<h3>标签译成中文</h3>
		<label class="check"
			><input type="checkbox" bind:checked={form.translation.enabled} />E-Hentai 的标签用 EhTagTranslation
			译成中文</label
		>
		<p class="adm-note">
			从 EhTagTranslation 的 Releases 下载 db.text.json，放进数据目录（{status?.translation.path ??
				'数据目录/db.text.json'}）。
		</p>
		{#if status?.translation.enabled}
			<p class="adm-note">
				{#if status.translation.error}
					<span class="bad">{status.translation.error}</span>
				{:else}
					已载入 {status.translation.entries} 条翻译（{status.translation.path}）
				{/if}
			</p>
		{/if}

		<h3>Bangumi</h3>
		<p class="adm-note">按系列查（用 series 标签，没有时用去掉卷号的标题），加作者、出版社、连载杂志、简介和用户标签，不改标题。</p>
		<label class="check"
			><input type="checkbox" bind:checked={form.bangumi.auto} />新书放进后台队列，每本之间隔 1 秒</label
		>
		<label class="check"
			><input type="checkbox" bind:checked={form.bangumi.review} />查到的先放进候选，确认后才写入</label
		>
		<label class="check"
			><input type="checkbox" bind:checked={form.bangumi.prefer_chinese} />系列名用中文名</label
		>
		<label class="field">
			<span class="adm-label">最多加几个用户标签</span>
			<input type="number" min="0" max="100" bind:value={form.bangumi.tag_limit} />
		</label>
		<label class="field">
			<span class="adm-label"
				>访问令牌（可选，查 NSFW 条目要它）· {form.bangumi.token_set ? '已设置，留空不改' : '未设置'}</span
			>
			<span class="cookie">
				<input type="password" autocomplete="off" bind:value={token} />
				{#if form.bangumi.token_set}
					<button class="adm-button" disabled={saving} onclick={() => save('token')}>清除</button>
				{/if}
			</span>
		</label>

		<h3>AniList</h3>
		<p class="adm-note">按系列查，加作者、类型、标签和简介，不改标题。条目以英文和罗马字为主，中文书名多半搜不到。</p>
		<label class="check"
			><input type="checkbox" bind:checked={form.anilist.auto} />新书放进后台队列，每本之间隔 2 秒</label
		>
		<label class="check"
			><input type="checkbox" bind:checked={form.anilist.review} />查到的先放进候选，确认后才写入</label
		>
		<div class="option">
			<span class="adm-label">系列名用</span>
			<div class="adm-tabs" role="group" aria-label="系列名用">
				{#each [['native', '原名'], ['romaji', '罗马字'], ['english', '英文']] as [key, label] (key)}
					<button
						class="adm-tab"
						aria-pressed={form.anilist.title === key}
						onclick={() => form && (form.anilist.title = key as MetadataSettings['anilist']['title'])}
						>{label}</button
					>
				{/each}
			</div>
		</div>
		<label class="check"
			><input type="checkbox" bind:checked={form.anilist.native_names} />作者名用原文写法</label
		>
		<label class="check"><input type="checkbox" bind:checked={form.anilist.spoilers} />也要剧透标签</label>
		<label class="field">
			<span class="adm-label">标签的最低相关度（0 到 100）</span>
			<input type="number" min="0" max="100" bind:value={form.anilist.min_tag_rank} />
		</label>

		<div class="adm-actions">
			<button class="adm-button" disabled={saving} onclick={() => save()}>{saving ? '保存中…' : '保存'}</button>
			{#if message}<span class="adm-note" role="status">{message}</span>{/if}
		</div>
	{/if}
	{#if error}<p class="bad" role="alert">{error}</p>{/if}

	{#if statusError}
		<p class="bad" role="alert">
			读不到队列状态：{statusError}{status ? '。下面是上一次读到的，可能已经过期' : ''}
		</p>
	{/if}
	{#if status}
		<h3>在线来源的队列</h3>
		<p class="adm-note">
			队列里的书不管自动运行开没开都会查。候选是“先放候选”时查到、等确认的结果，用接口或 MCP 读取、改写后批准。
		</p>
		{#each providers as [provider, label] (provider)}
			{@const queue = status.providers[provider]}
			<div class="adm-row">
				<span
					>{label}：排队 {queue.queued} 本 · 候选 {queue.candidates} 条 · 本次启动后完成 {queue.done}、失败
					{queue.failed}</span
				>
				<span class="adm-actions">
					{#if queue.paused}
						<button class="adm-button" onclick={() => resume(provider)}>继续</button>
					{/if}
					<button
						class="adm-button danger"
						disabled={queue.queued === 0}
						onclick={() => clearQueue(provider, label)}>清空…</button
					>
				</span>
			</div>
			{#if queue.paused}
				<p class="bad">已暂停：{queue.paused}</p>
			{:else if queue.current !== null}
				<p class="adm-note">正在查 <a href="/a/{queue.current}">#{queue.current}</a></p>
			{/if}
			{#if queue.last_error}
				<p class="adm-note">最近一次失败：{queue.last_error}</p>
			{/if}
		{/each}
	{/if}
</section>

<style>
	h3 {
		margin: 0.6rem 0 0;
		padding-top: 0.6rem;
		border-top: 1px solid var(--border);
		font-size: 0.85rem;
	}

	.check {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		font-size: 0.85rem;
	}

	.field {
		display: grid;
		gap: 0.3rem;
	}

	.field input {
		box-sizing: border-box;
		width: 100%;
		padding: 0.4rem 0.6rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: var(--sunken);
		color: var(--fg);
		font-size: 0.85rem;
	}

	.option {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.6rem;
	}

	.cookies {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(min(100%, 16rem), 1fr));
		gap: 0.6rem;
	}

	.cookie {
		display: flex;
		gap: 0.4rem;
	}

	.bad {
		margin: 0;
		color: var(--danger);
		font-size: 0.8rem;
	}
</style>
