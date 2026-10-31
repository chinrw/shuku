<script lang="ts">
	// 控制台设置页的“元数据”卡片：文件名、ComicInfo、E-Hentai 和中文翻译的选项，
	// 以及后台 E-Hentai 队列的状态。cookie 只写不读，输入框留空表示不改。
	import { onMount } from 'svelte';
	import {
		clearMetadataQueue,
		metadataSettings,
		metadataStatus,
		resumeMetadataQueue,
		saveMetadataSettings,
		type CookieName,
		type MetadataSettings,
		type MetadataStatus
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

	async function save(clear?: CookieName) {
		if (!form || saving) return;
		saving = true;
		message = '';
		error = '';
		const { cookies_set: _, ...ehentai } = form.ehentai;
		const typed = Object.fromEntries(
			Object.entries(cookies).filter(([, value]) => value.trim() !== '')
		) as Partial<Record<CookieName, string>>;
		if (clear) typed[clear] = '';
		try {
			form = await saveMetadataSettings({
				replace_title: form.replace_title,
				filename: form.filename,
				comicinfo: form.comicinfo,
				ehentai: { ...ehentai, cookies: typed },
				translation: form.translation
			});
			cookies = { ipb_member_id: '', ipb_pass_hash: '', star: '', igneous: '' };
			message = '已保存';
			await refreshStatus();
		} catch (cause) {
			error = `没有保存：${describe(cause)}`;
		} finally {
			saving = false;
		}
	}

	async function resume() {
		await resumeMetadataQueue().catch((cause) => (error = describe(cause)));
		await refreshStatus();
	}

	async function clearQueue() {
		if (!confirm('清空队列？队列里的书不会再自动查 E-Hentai。')) return;
		await clearMetadataQueue().catch((cause) => (error = describe(cause)));
		await refreshStatus();
	}
</script>

<section class="adm-card">
	<h2>元数据</h2>
	<p class="adm-note">
		代替 LRR 的文件名解析、ComicInfo、E-Hentai 和 ETagCN 插件。只给扫描新加入的书自动运行，从 LRR
		迁移来的书不动；详情页的“获取元数据”可以对单本手动运行。和 LRR 一样只加标签，不删已有的。
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
		<h3>E-Hentai 队列</h3>
		<div class="adm-row">
			<span>排队 {status.queue.length} 本 · 本次启动后完成 {status.queue.done}、失败 {status.queue.failed}</span>
			<span class="adm-actions">
				{#if status.queue.paused}
					<button class="adm-button" onclick={resume}>继续</button>
				{/if}
				<button class="adm-button danger" disabled={status.queue.length === 0} onclick={clearQueue}>清空…</button>
			</span>
		</div>
		{#if status.queue.paused}
			<p class="bad">已暂停：{status.queue.paused}</p>
		{:else if !status.queue.enabled && status.queue.length > 0}
			<p class="adm-note">自动运行关着，队列不处理。</p>
		{:else if status.queue.current !== null}
			<p class="adm-note">正在查 <a href="/a/{status.queue.current}">#{status.queue.current}</a></p>
		{/if}
		{#if status.queue.last_error}
			<p class="adm-note">最近一次失败：{status.queue.last_error}</p>
		{/if}
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
