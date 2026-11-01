<script lang="ts">
	// 控制台设置页的“元数据来源”：文件名、ComicInfo、E-Hentai 和中文翻译、Bangumi、AniList 的选项。
	// 每个来源一张可折叠的卡片，全部改完一起保存。队列和候选在任务页（MetadataQueues）。
	// cookie 和令牌只写不读，输入框留空表示不改。
	import { onMount } from 'svelte';
	import {
		metadataSettings,
		metadataStatus,
		saveMetadataSettings,
		type CookieName,
		type MetadataSettings,
		type MetadataStatus
	} from '#lib/admin.ts';
	import { ApiError } from '#lib/api.ts';
	import CardHead from './CardHead.svelte';
	import Check from './Check.svelte';
	import Setting from './Setting.svelte';

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
	let message = $state('');
	let error = $state('');

	const cookieFields: [CookieName, string][] = [
		['ipb_member_id', 'ipb_member_id'],
		['ipb_pass_hash', 'ipb_pass_hash'],
		['star', 'star（可选）'],
		['igneous', 'igneous（可选，ExHentai 用）']
	];

	const onOff = (value: boolean) => (value ? '开' : '关');

	function describe(cause: unknown) {
		return cause instanceof ApiError ? cause.message : '网络错误';
	}

	/** 这里只用状态里的翻译表载入结果，保存后重读一次就够，不用轮询。 */
	async function refreshStatus() {
		status = await metadataStatus().catch(() => status);
	}

	onMount(() => {
		metadataSettings()
			.then((settings) => (form = settings))
			.catch((cause) => (error = `读取设置失败：${describe(cause)}`));
		refreshStatus();
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
</script>

<section class="adm-card">
	<CardHead
		title="元数据来源"
		sub="所有设备共用"
		help="代替 LRR 的文件名解析、ComicInfo、E-Hentai 和 ETagCN 插件，另有给按卷出版的漫画用的 Bangumi 和 AniList。只给扫描新加入的书自动运行，从 LRR 迁移来的书不动；详情页的“获取元数据”可以对单本手动运行。和 LRR 一样只加标签，不删已有的；来源和简介只在书还没有时写入。同人志和普通漫画各跑一个实例，这里的开关各配各的。"
	/>
	{#if form}
		<Check
			bind:checked={form.replace_title}
			label="用找到的标题替换原标题"
			help="对应 LRR 的“允许插件替换标题”。Bangumi 和 AniList 查的是系列，不给标题。"
		/>

		<details class="adm-source">
			<summary>
				<span class="name">文件名</span>
				<span class="adm-sub">自动 {onOff(form.filename.auto)}</span>
			</summary>
			<Check bind:checked={form.filename.auto} label="新书自动运行" />
			<details class="adm-advanced">
				<summary>高级</summary>
				<Check bind:checked={form.filename.trailing_tags} label={'末尾的 {a, b} 当作标签'} />
				<Check
					bind:checked={form.filename.keep_all_captures}
					label="其他括号里的内容放进 parsed: 命名空间"
				/>
				<Setting
					label="正则"
					id="filename-regex"
					help="Rust 语法，命名分组决定命名空间；留空用默认的同人志格式。"
				>
					<input
						id="filename-regex"
						class="adm-input adm-mono"
						bind:value={form.filename.regex}
						placeholder="默认"
						spellcheck="false"
					/>
				</Setting>
			</details>
		</details>

		<details class="adm-source">
			<summary>
				<span class="name">ComicInfo.xml</span>
				<span class="adm-sub">自动 {onOff(form.comicinfo.auto)}</span>
			</summary>
			<Check bind:checked={form.comicinfo.auto} label="新书自动运行" />
		</details>

		<details class="adm-source">
			<summary>
				<span class="name">E-Hentai</span>
				<span class="adm-sub"
					>自动 {onOff(form.ehentai.auto)} · 先放候选 {onOff(form.ehentai.review)} · 译中文 {onOff(
						form.translation.enabled
					)}</span
				>
			</summary>
			<Check bind:checked={form.ehentai.auto} label="新书放进后台队列" help="每本之间隔 4 秒。" />
			<Check
				bind:checked={form.ehentai.review}
				label="查到的先放进候选，确认后才写入"
				help="候选在任务页的队列里能看到数量，用接口或 MCP 读取、改写后批准。"
			/>
			<Check
				bind:checked={form.translation.enabled}
				label="标签用 EhTagTranslation 译成中文"
				help="从 EhTagTranslation 的 Releases 下载 db.text.json，放进数据目录（{status?.translation
					.path ?? '数据目录/db.text.json'}）。"
			/>
			{#if status?.translation.enabled}
				<p class="adm-note">
					{#if status.translation.error}
						<span class="bad">{status.translation.error}</span>
					{:else}
						已载入 {status.translation.entries} 条翻译（{status.translation.path}）
					{/if}
				</p>
			{/if}
			<h4>账号</h4>
			<div class="cookies">
				{#each cookieFields as [name, label] (name)}
					<label class="field">
						<span class="adm-label"
							>{label} · {form.ehentai.cookies_set[name] ? '已设置，留空不改' : '未设置'}</span
						>
						<span class="cookie">
							<input class="adm-input" type="password" autocomplete="off" bind:value={cookies[name]} />
							{#if form.ehentai.cookies_set[name]}
								<button class="adm-button" disabled={saving} onclick={() => save(name)}>清除</button>
							{/if}
						</span>
					</label>
				{/each}
			</div>
			<details class="adm-advanced">
				<summary>高级</summary>
				<Check bind:checked={form.ehentai.exhentai} label="在 ExHentai 上搜索" />
				<Check bind:checked={form.ehentai.use_thumbnail} label="先用封面图搜索" />
				<Check bind:checked={form.ehentai.search_gid} label="标题里有 [画廊 ID] 时先按 ID 搜索" />
				<Check bind:checked={form.ehentai.japanese_title} label="有日文标题时用日文标题" />
				<Check bind:checked={form.ehentai.extra_tags} label="另加上传者和发布时间标签" />
				<Check bind:checked={form.ehentai.expunged} label="也搜索已删除的画廊" />
				<Setting label="搜索限定语言" id="eh-language" help="例如 chinese，留空不限。">
					<input id="eh-language" class="adm-input" bind:value={form.ehentai.language} spellcheck="false" />
				</Setting>
			</details>
		</details>

		<details class="adm-source">
			<summary>
				<span class="name">Bangumi</span>
				<span class="adm-sub">自动 {onOff(form.bangumi.auto)} · 先放候选 {onOff(form.bangumi.review)}</span>
			</summary>
			<Check
				bind:checked={form.bangumi.auto}
				label="新书放进后台队列"
				help="按系列查（用 series 标签，没有时用去掉卷号的标题），加作者、出版社、连载杂志、简介和用户标签，不改标题。每本之间隔 1 秒。"
			/>
			<Check bind:checked={form.bangumi.review} label="查到的先放进候选，确认后才写入" />
			<h4>账号</h4>
			<label class="field">
				<span class="adm-label"
					>访问令牌（可选，查 NSFW 条目要它）· {form.bangumi.token_set ? '已设置，留空不改' : '未设置'}</span
				>
				<span class="cookie">
					<input class="adm-input" type="password" autocomplete="off" bind:value={token} />
					{#if form.bangumi.token_set}
						<button class="adm-button" disabled={saving} onclick={() => save('token')}>清除</button>
					{/if}
				</span>
			</label>
			<details class="adm-advanced">
				<summary>高级</summary>
				<Check bind:checked={form.bangumi.prefer_chinese} label="系列名用中文名" />
				<Setting label="最多加几个用户标签" id="bangumi-tags">
					<input
						id="bangumi-tags"
						class="adm-input"
						type="number"
						min="0"
						max="100"
						bind:value={form.bangumi.tag_limit}
					/>
				</Setting>
			</details>
		</details>

		<details class="adm-source">
			<summary>
				<span class="name">AniList</span>
				<span class="adm-sub">自动 {onOff(form.anilist.auto)} · 先放候选 {onOff(form.anilist.review)}</span>
			</summary>
			<Check
				bind:checked={form.anilist.auto}
				label="新书放进后台队列"
				help="按系列查，加作者、类型、标签和简介，不改标题。条目以英文和罗马字为主，中文书名多半搜不到。每本之间隔 2 秒。"
			/>
			<Check bind:checked={form.anilist.review} label="查到的先放进候选，确认后才写入" />
			<details class="adm-advanced">
				<summary>高级</summary>
				<Setting label="系列名用">
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
				</Setting>
				<Check bind:checked={form.anilist.native_names} label="作者名用原文写法" />
				<Check bind:checked={form.anilist.spoilers} label="也要剧透标签" />
				<Setting label="标签的最低相关度" id="anilist-rank" help="0 到 100，AniList 给每个标签的相关度打分。">
					<input
						id="anilist-rank"
						class="adm-input"
						type="number"
						min="0"
						max="100"
						bind:value={form.anilist.min_tag_rank}
					/>
				</Setting>
			</details>
		</details>

		<div class="adm-actions">
			<button class="adm-button primary" disabled={saving} onclick={() => save()}
				>{saving ? '保存中…' : '保存'}</button
			>
			{#if message}<span class="adm-note" role="status">{message}</span>{/if}
		</div>
	{/if}
	{#if error}<p class="bad" role="alert">{error}</p>{/if}
</section>

<style>
	h4 {
		margin: 0.5rem 0 0;
		color: var(--muted);
		font-size: 0.78rem;
		font-weight: 600;
	}

	.field {
		display: grid;
		gap: 0.3rem;
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
