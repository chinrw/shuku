<script lang="ts">
	import {
		scanSettings,
		serverSettings,
		updateScanSettings,
		updateServerSettings,
		type ScanSettings,
		type ServerSettings
	} from '#lib/admin.ts';
	import { adminStatus } from '#lib/adminStatus.svelte.ts';
	import { page } from '$app/state';
	import CardHead from './CardHead.svelte';
	import MetadataSettings from './MetadataSettings.svelte';
	import Setting from './Setting.svelte';
	import TokenSettings from './TokenSettings.svelte';
	import ThemePicker from '#lib/ThemePicker.svelte';
	import {
		MOTIONS,
		settings,
		type Direction,
		type LibraryView,
		type ReaderMode
	} from '#lib/settings.svelte.ts';

	const prefs = settings();

	// 设置按用途分成几页，地址里的 #分区 决定显示哪一页，可以直接链接。
	const sections = [
		['scan', '扫描'],
		['metadata', '元数据来源'],
		['display', '阅读与显示'],
		['access', '网络与安全'],
		['about', '关于']
	] as const;
	type Section = (typeof sections)[number][0];
	let section = $derived<Section>(
		sections.find(([key]) => `#${key}` === page.url.hash)?.[0] ?? 'scan'
	);
	let server = $derived(adminStatus.current?.server);

	let scan = $state<ScanSettings | null>(null);
	let general = $state<ServerSettings | null>(null);
	// 逗号分隔的输入框，保存时再拆成列表。
	let skipText = $state('');
	let hostsText = $state('');
	let nameText = $state('');
	let saving = $state(false);
	let error = $state<string | null>(null);

	const message = (cause: unknown) => (cause instanceof Error ? cause.message : String(cause));
	const split = (text: string) =>
		text
			.split(/[,，]/)
			.map((item) => item.trim())
			.filter(Boolean);

	$effect(() => {
		scanSettings()
			.then((loaded) => {
				scan = loaded;
				skipText = loaded.extra_skip.join(', ');
			})
			.catch((cause) => (error = message(cause)));
		serverSettings()
			.then((loaded) => {
				general = loaded;
				hostsText = loaded.allowed_hosts.join(', ');
				nameText = loaded.name;
			})
			.catch((cause) => (error = message(cause)));
	});

	// 立即保存；失败时保留原值并显示原因。
	async function save(action: () => Promise<void>) {
		saving = true;
		error = null;
		try {
			await action();
		} catch (cause) {
			error = message(cause);
		} finally {
			saving = false;
		}
	}

	const changeScan = (patch: Partial<ScanSettings>) =>
		save(async () => {
			scan = await updateScanSettings(patch);
			skipText = scan.extra_skip.join(', ');
		});
	const changeGeneral = (patch: Partial<ServerSettings>) =>
		save(async () => {
			general = await updateServerSettings(patch);
			hostsText = general.allowed_hosts.join(', ');
			nameText = general.name;
		});

	// 定时扫描是文件监视的后备，默认不开，间隔都比较长。
	const intervals: [number, string][] = [
		[0, '不定时'],
		[24, '每天'],
		[72, '每 3 天'],
		[168, '每周'],
		[720, '每 30 天']
	];

	function set<K extends 'view' | 'mode' | 'direction' | 'spread' | 'motion'>(key: K, value: (typeof prefs)[K]) {
		prefs[key] = value;
		prefs.save();
	}

	const views: [LibraryView, string][] = [
		['grid', '封面'],
		['list', '列表']
	];
	const modes: [ReaderMode, string][] = [
		['paged', '翻页'],
		['scroll', '滚动']
	];
	const directions: [Direction, string][] = [
		['rtl', '从右往左'],
		['ltr', '从左往右']
	];
</script>

<div class="adm-head">
	<h1>设置</h1>
</div>

<nav class="adm-tabs sections" aria-label="设置分区">
	{#each sections as [key, label] (key)}
		<a class="adm-tab" href="#{key}" aria-current={section === key ? 'page' : undefined}>{label}</a>
	{/each}
</nav>

<div class="adm-stack">
	{#if error}<p class="error">保存失败：{error}</p>{/if}

	{#if section === 'scan'}
		<section class="adm-card">
			<CardHead
				title="扫描"
				sub="所有设备共用"
				help="只扫 zip、cbz 和散放图片的文件夹，别的文件只计数。设置存在服务端，下一次扫描起生效。"
			/>
			{#if scan}
				<Setting
					label="自动扫描新文件"
					help="监视书库目录，有文件变化后安静 10 秒自动扫描。书库在网络盘上监视不到变化时关掉，改用定时扫描。"
				>
					<div class="adm-tabs" role="group" aria-label="自动扫描新文件">
						<button
							class="adm-tab"
							aria-pressed={scan.watch}
							disabled={saving}
							onclick={() => changeScan({ watch: true })}>开</button
						>
						<button
							class="adm-tab"
							aria-pressed={!scan.watch}
							disabled={saving}
							onclick={() => changeScan({ watch: false })}>关</button
						>
					</div>
				</Setting>
				<Setting
					label="定时扫描"
					help="从上一次扫描结束算起，给文件监视漏掉的变化兜底。服务启动时总会扫描一次。"
				>
					<div class="adm-tabs" role="group" aria-label="定时扫描">
						{#each intervals as [hours, label] (hours)}
							<button
								class="adm-tab"
								aria-pressed={scan.scan_interval_hours === hours}
								disabled={saving}
								onclick={() => changeScan({ scan_interval_hours: hours })}>{label}</button
							>
						{/each}
					</div>
				</Setting>
				<Setting
					label="系列"
					help="按卷号认时，名字只有卷号或话数（VOL_1、032、第10話、番外1）的书归进上一层文件夹的系列。第一级目录为一部作品时，书库目录下每个第一级文件夹就是一部作品，里面的书不管叫什么都归进去；适合普通漫画书库，同人志书库别开。下一次扫描起对还没归进系列的书生效。"
				>
					<div class="adm-tabs" role="group" aria-label="系列">
						<button
							class="adm-tab"
							aria-pressed={!scan.series_from_top_dir}
							disabled={saving}
							onclick={() => changeScan({ series_from_top_dir: false })}>按卷号认</button
						>
						<button
							class="adm-tab"
							aria-pressed={scan.series_from_top_dir}
							disabled={saving}
							onclick={() => changeScan({ series_from_top_dir: true })}>第一级目录为一部作品</button
						>
					</div>
				</Setting>
				<details class="adm-advanced">
					<summary>高级</summary>
					<Setting
						label="隐藏目录"
						help="以 . 开头的目录和文件通常是下载工具的临时目录，往往只有那个程序自己能读。"
					>
						<div class="adm-tabs" role="group" aria-label="隐藏目录">
							<button
								class="adm-tab"
								aria-pressed={!scan.scan_hidden}
								disabled={saving}
								onclick={() => changeScan({ scan_hidden: false })}>跳过</button
							>
							<button
								class="adm-tab"
								aria-pressed={scan.scan_hidden}
								disabled={saving}
								onclick={() => changeScan({ scan_hidden: true })}>扫描</button
							>
						</div>
					</Setting>
					<Setting
						label="额外跳过"
						id="extra-skip"
						help="目录或文件名，逗号分隔，可以用 * 通配，不分大小写。对文件夹里的单张图片不起作用，要跳过整本就写文件夹名。跳过的地方已入库的书保持原样。"
					>
						<form
							class="field"
							onsubmit={(event) => {
								event.preventDefault();
								changeScan({ extra_skip: split(skipText) });
							}}
						>
							<input
								id="extra-skip"
								bind:value={skipText}
								placeholder="@eaDir, *.part"
								autocapitalize="off"
								spellcheck="false"
							/>
							<button class="adm-button" disabled={saving}>保存</button>
						</form>
					</Setting>
				</details>
			{/if}
		</section>
	{:else if section === 'metadata'}
		<MetadataSettings />
	{:else if section === 'display'}
		<section class="adm-card">
			<CardHead title="书库" sub="所有设备共用" />
			{#if general}
				<Setting
					label="书库名"
					id="library-name"
					help="显示在书库页左上角。同一台机器上开了几个实例时，桌面和手机客户端用它区分服务器。"
				>
					<form
						class="field"
						onsubmit={(event) => {
							event.preventDefault();
							changeGeneral({ name: nameText });
						}}
					>
						<input id="library-name" bind:value={nameText} placeholder="书库" maxlength="100" />
						<button class="adm-button" disabled={saving}>保存</button>
					</form>
				</Setting>
				<Setting
					label="条漫判断"
					help="按页面宽高比判断条漫并改用上下连续滚动。关闭后只认阅读器里手动打开的条漫开关。"
				>
					<div class="adm-tabs" role="group" aria-label="条漫判断">
						<button
							class="adm-tab"
							aria-pressed={general.detect_webtoons}
							disabled={saving}
							onclick={() => changeGeneral({ detect_webtoons: true })}>自动</button
						>
						<button
							class="adm-tab"
							aria-pressed={!general.detect_webtoons}
							disabled={saving}
							onclick={() => changeGeneral({ detect_webtoons: false })}>关闭</button
						>
					</div>
				</Setting>
			{/if}
		</section>

		<section class="adm-card">
			<CardHead title="这台设备" sub="存在浏览器里，只影响当前设备" />
			<Setting
				label="配色"
				help="跟随系统时，系统是深色就用“深色时用”那一套，浅色就用“浅色时用”那一套。阅读器的设置里也能改。"
			>
				<ThemePicker />
			</Setting>
			<Setting
				label="滑动动画"
				help="面板滑入和翻页时两屏左右滑动。跟随系统时，系统设置了减少动态效果就不播放；开则总是播放。"
			>
				<div class="adm-tabs" role="group" aria-label="滑动动画">
					{#each MOTIONS as [key, label] (key)}
						<button class="adm-tab" aria-pressed={prefs.motion === key} onclick={() => set('motion', key)}
							>{label}</button
						>
					{/each}
				</div>
			</Setting>
			<Setting label="书库视图" help="书库页默认显示封面网格还是列表。">
				<div class="adm-tabs" role="group" aria-label="书库视图">
					{#each views as [key, label] (key)}
						<button class="adm-tab" aria-pressed={prefs.view === key} onclick={() => set('view', key)}
							>{label}</button
						>
					{/each}
				</div>
			</Setting>
			<Setting label="阅读方式" help="翻页是一屏一屏地看，滚动是上下连续滚。条漫总是上下滚动。">
				<div class="adm-tabs" role="group" aria-label="阅读方式">
					{#each modes as [key, label] (key)}
						<button class="adm-tab" aria-pressed={prefs.mode === key} onclick={() => set('mode', key)}
							>{label}</button
						>
					{/each}
				</div>
			</Setting>
			<Setting label="翻页方向" help="日本漫画一般从右往左翻。">
				<div class="adm-tabs" role="group" aria-label="翻页方向">
					{#each directions as [key, label] (key)}
						<button
							class="adm-tab"
							aria-pressed={prefs.direction === key}
							onclick={() => set('direction', key)}>{label}</button
						>
					{/each}
				</div>
			</Setting>
			<Setting label="横屏双页" help="屏幕横着时一次并排显示两页。">
				<div class="adm-tabs" role="group" aria-label="横屏双页">
					<button class="adm-tab" aria-pressed={prefs.spread} onclick={() => set('spread', true)}>开</button>
					<button class="adm-tab" aria-pressed={!prefs.spread} onclick={() => set('spread', false)}>关</button>
				</div>
			</Setting>
		</section>
	{:else if section === 'access'}
		<section class="adm-card">
			<CardHead title="访问" sub="所有设备共用" />
			{#if general}
				<Setting
					label="允许的域名"
					id="allowed-hosts"
					help="IP、localhost 和不带点的主机名（例如 nas）总是允许；用带点的域名访问时把它加在这里，逗号分隔。用来防别的网页借域名读书库。"
				>
					<form
						class="field"
						onsubmit={(event) => {
							event.preventDefault();
							changeGeneral({ allowed_hosts: split(hostsText) });
						}}
					>
						<input
							id="allowed-hosts"
							bind:value={hostsText}
							placeholder="nas.example.ts.net"
							autocapitalize="off"
							spellcheck="false"
						/>
						<button class="adm-button" disabled={saving}>保存</button>
					</form>
				</Setting>
			{/if}
		</section>

		<TokenSettings />
	{:else}
		<section class="adm-card">
			<CardHead
				title="启动参数"
				help="用命令行参数或环境变量设置，这里只读，改了要重启服务。"
			/>
			{#if server}
				<dl class="adm-kv left">
					<dt>内容目录</dt>
					<dd class="adm-mono">{server.content_dir}</dd>
					<dt>数据目录</dt>
					<dd class="adm-mono">{server.data_dir}</dd>
					<dt>网页目录</dt>
					<dd class="adm-mono">{server.web_dir}</dd>
					<dt>监听地址</dt>
					<dd class="adm-mono">{server.listen}</dd>
					<dt>版本</dt>
					<dd>Shuku {server.version}</dd>
				</dl>
			{/if}
		</section>
	{/if}
</div>

<style>
	.sections {
		margin-bottom: 1rem;
	}

	.field {
		display: flex;
		gap: 0.5rem;
		min-width: 0;
	}

	.field input {
		flex: 1;
		min-width: 0;
		height: 2.5rem;
		padding: 0 0.7rem;
		border: 1px solid var(--border);
		border-radius: 8px;
		background: var(--bg);
	}
</style>
