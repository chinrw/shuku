<script lang="ts">
	import { serverSettings, updateServerSettings, type ServerSettings } from '#lib/admin.ts';
	import { adminStatus } from '#lib/adminStatus.svelte.ts';
	import MetadataSettings from './MetadataSettings.svelte';
	import { settings, type Direction, type LibraryView, type ReaderMode } from '#lib/settings.svelte.ts';

	const prefs = settings();
	let server = $derived(adminStatus.current?.server);

	let stored = $state<ServerSettings | null>(null);
	let saving = $state(false);
	let error = $state<string | null>(null);

	$effect(() => {
		serverSettings()
			.then((loaded) => (stored = loaded))
			.catch((cause) => (error = cause instanceof Error ? cause.message : String(cause)));
	});

	// 点一下立即保存；失败时保留原值并显示原因。
	async function change(patch: Partial<ServerSettings>) {
		saving = true;
		error = null;
		try {
			stored = await updateServerSettings(patch);
		} catch (cause) {
			error = cause instanceof Error ? cause.message : String(cause);
		} finally {
			saving = false;
		}
	}

	const intervals: [number, string][] = [
		[0, '不定时'],
		[6, '每 6 小时'],
		[24, '每天'],
		[72, '每 3 天'],
		[168, '每周']
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
	<span class="adm-sub">设备令牌和插件在 M2 加到这里</span>
</div>

<div class="adm-stack">
	<section class="adm-card">
		<h2>扫描</h2>
		<p class="adm-note">存在服务端，所有设备共用，改了立即生效，下一次扫描起按新设置。</p>
		{#if error}<p class="error">保存失败：{error}</p>{/if}
		{#if stored}
			<div class="option">
				<span class="adm-label">隐藏目录</span>
				<div class="adm-tabs" role="group" aria-label="隐藏目录">
					<button
						class="adm-tab"
						aria-pressed={!stored.scan_hidden}
						disabled={saving}
						onclick={() => change({ scan_hidden: false })}>跳过</button
					>
					<button
						class="adm-tab"
						aria-pressed={stored.scan_hidden}
						disabled={saving}
						onclick={() => change({ scan_hidden: true })}>扫描</button
					>
				</div>
			</div>
			<p class="adm-note">以 . 开头的目录和文件通常是下载工具的临时目录，往往只有那个程序自己能读。</p>
			<div class="option">
				<span class="adm-label">定时扫描</span>
				<div class="adm-tabs" role="group" aria-label="定时扫描">
					{#each intervals as [hours, label] (hours)}
						<button
							class="adm-tab"
							aria-pressed={stored.scan_interval_hours === hours}
							disabled={saving}
							onclick={() => change({ scan_interval_hours: hours })}>{label}</button
						>
					{/each}
				</div>
			</div>
			<p class="adm-note">从上一次扫描结束算起。服务启动时总会扫描一次，扫描结束后接着补算指纹。</p>
		{/if}
	</section>

	<section class="adm-card">
		<h2>服务端</h2>
		<p class="adm-note">用命令行参数或环境变量设置，改了要重启服务。</p>
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
				<dt>允许的主机名</dt>
				<dd>
					IP、localhost 和不带点的主机名{#if server.allowed_hosts.length > 0}，以及
						<span class="adm-mono">{server.allowed_hosts.join('、')}</span>{/if}（<code>--allowed-host</code>）
				</dd>
				<dt>条漫判断</dt>
				<dd>
					{server.detect_webtoons ? '按页面宽高比自动判断' : '关闭，只认阅读器里的手动开关'}（<code
						>--no-webtoon-detection</code
					>）
				</dd>
				<dt>版本</dt>
				<dd>Shuku {server.version}</dd>
			</dl>
		{/if}
	</section>

	<MetadataSettings />

	<section class="adm-card">
		<h2>这台设备</h2>
		<p class="adm-note">存在浏览器里，只影响当前设备。</p>
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
	.option {
		display: grid;
		grid-template-columns: minmax(6rem, 10rem) minmax(0, 1fr);
		align-items: center;
		gap: 0.75rem;
		padding: 0.6rem 0;
		border-top: 1px solid var(--border);
	}
</style>
