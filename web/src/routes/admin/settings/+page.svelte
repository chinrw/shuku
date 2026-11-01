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
	import MetadataSettings from './MetadataSettings.svelte';
	import TokenSettings from './TokenSettings.svelte';
	import ThemePicker from '#lib/ThemePicker.svelte';
	import { settings, type Direction, type LibraryView, type ReaderMode } from '#lib/settings.svelte.ts';

	const prefs = settings();
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

	function set<K extends 'view' | 'mode' | 'direction' | 'spread'>(key: K, value: (typeof prefs)[K]) {
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

<div class="adm-stack">
	{#if error}<p class="error">保存失败：{error}</p>{/if}
	<section class="adm-card">
		<h2>扫描</h2>
		<p class="adm-note">
			只扫 zip、cbz 和散放图片的文件夹，别的文件只计数。存在服务端，所有设备共用，下一次扫描起生效。
		</p>
		{#if scan}
			<div class="option">
				<span class="adm-label">自动扫描新文件</span>
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
			</div>
			<p class="adm-note">
				监视书库目录，有文件变化后安静 10 秒自动扫描。书库在网络盘上监视不到变化时关掉，改用下面的定时扫描。
			</p>
			<div class="option">
				<span class="adm-label">隐藏目录</span>
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
			</div>
			<p class="adm-note">以 . 开头的目录和文件通常是下载工具的临时目录，往往只有那个程序自己能读。</p>
			<form
				class="option"
				onsubmit={(event) => {
					event.preventDefault();
					changeScan({ extra_skip: split(skipText) });
				}}
			>
				<label class="adm-label" for="extra-skip">额外跳过</label>
				<div class="field">
					<input
						id="extra-skip"
						bind:value={skipText}
						placeholder="@eaDir, *.part"
						autocapitalize="off"
						spellcheck="false"
					/>
					<button class="adm-button" disabled={saving}>保存</button>
				</div>
			</form>
			<p class="adm-note">目录或文件名，逗号分隔，可以用 * 通配，不分大小写。对文件夹里的单张图片不起作用，要跳过整本就写文件夹名。跳过的地方已入库的书保持原样。</p>
			<div class="option">
				<span class="adm-label">定时扫描</span>
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
			</div>
			<p class="adm-note">从上一次扫描结束算起，给文件监视漏掉的变化兜底。服务启动时总会扫描一次。</p>
			<div class="option">
				<span class="adm-label">系列</span>
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
			</div>
			<p class="adm-note">
				按卷号认时，名字只有卷号或话数（VOL_1、032、第10話、番外1）的书归进上一层文件夹的系列。第一级目录为一部作品时，书库目录下每个第一级文件夹就是一部作品，里面的书不管叫什么都归进去；适合普通漫画书库，同人志书库别开。下一次扫描起对还没归进系列的书生效。
			</p>
		{/if}
	</section>

	<section class="adm-card">
		<h2>阅读和访问</h2>
		{#if general}
			<form
				class="option"
				onsubmit={(event) => {
					event.preventDefault();
					changeGeneral({ name: nameText });
				}}
			>
				<label class="adm-label" for="library-name">书库名</label>
				<div class="field">
					<input id="library-name" bind:value={nameText} placeholder="书库" maxlength="100" />
					<button class="adm-button" disabled={saving}>保存</button>
				</div>
			</form>
			<p class="adm-note">显示在书库页左上角。同一台机器上开了几个实例时，桌面和手机客户端用它区分服务器。</p>
			<div class="option">
				<span class="adm-label">条漫判断</span>
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
			</div>
			<p class="adm-note">按页面宽高比判断条漫并改用上下连续滚动。关闭后只认阅读器里手动打开的条漫开关。</p>
			<form
				class="option"
				onsubmit={(event) => {
					event.preventDefault();
					changeGeneral({ allowed_hosts: split(hostsText) });
				}}
			>
				<label class="adm-label" for="allowed-hosts">允许的域名</label>
				<div class="field">
					<input
						id="allowed-hosts"
						bind:value={hostsText}
						placeholder="nas.example.ts.net"
						autocapitalize="off"
						spellcheck="false"
					/>
					<button class="adm-button" disabled={saving}>保存</button>
				</div>
			</form>
			<p class="adm-note">
				IP、localhost 和不带点的主机名（例如 nas）总是允许；用带点的域名访问时把它加在这里，逗号分隔。用来防别的网页借域名读书库。
			</p>
		{/if}
	</section>

	<section class="adm-card">
		<h2>启动参数</h2>
		<p class="adm-note">用命令行参数或环境变量设置，这里只读，改了要重启服务。</p>
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

	<MetadataSettings />

	<TokenSettings />

	<section class="adm-card">
		<h2>这台设备</h2>
		<p class="adm-note">存在浏览器里，只影响当前设备。</p>
		<div class="option">
			<span class="adm-label picker-label">配色</span>
			<ThemePicker />
		</div>
		<div class="option">
			<span class="adm-label">书库视图</span>
			<div class="adm-tabs" role="group" aria-label="书库视图">
				{#each views as [key, label] (key)}
					<button class="adm-tab" aria-pressed={prefs.view === key} onclick={() => set('view', key)}>{label}</button>
				{/each}
			</div>
		</div>
		<div class="option">
			<span class="adm-label">阅读方式</span>
			<div class="adm-tabs" role="group" aria-label="阅读方式">
				{#each modes as [key, label] (key)}
					<button class="adm-tab" aria-pressed={prefs.mode === key} onclick={() => set('mode', key)}>{label}</button>
				{/each}
			</div>
		</div>
		<div class="option">
			<span class="adm-label">翻页方向</span>
			<div class="adm-tabs" role="group" aria-label="翻页方向">
				{#each directions as [key, label] (key)}
					<button class="adm-tab" aria-pressed={prefs.direction === key} onclick={() => set('direction', key)}>{label}</button>
				{/each}
			</div>
		</div>
		<div class="option">
			<span class="adm-label">横屏双页</span>
			<div class="adm-tabs" role="group" aria-label="横屏双页">
				<button class="adm-tab" aria-pressed={prefs.spread} onclick={() => set('spread', true)}>开</button>
				<button class="adm-tab" aria-pressed={!prefs.spread} onclick={() => set('spread', false)}>关</button>
			</div>
		</div>
	</section>
</div>

<style>
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

	/* 跟随系统时配色下面还有两行，标签对齐第一行。 */
	.picker-label {
		align-self: start;
		padding-top: 0.35rem;
	}

	.option {
		display: grid;
		grid-template-columns: minmax(6rem, 10rem) minmax(0, 1fr);
		align-items: center;
		gap: 0.75rem;
		padding: 0.6rem 0;
		border-top: 1px solid var(--border);
	}
</style>
