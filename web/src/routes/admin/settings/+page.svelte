<script lang="ts">
	import { adminStatus } from '#lib/adminStatus.svelte.ts';
	import { settings, type Direction, type LibraryView, type ReaderMode } from '#lib/settings.svelte.ts';

	const prefs = settings();
	let server = $derived(adminStatus.current?.server);

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
	<span class="adm-sub">M2 起可以在这里改服务端设置、管理设备令牌和插件</span>
</div>

<div class="adm-stack">
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
				<dt>版本</dt>
				<dd>Shuku {server.version}</dd>
			</dl>
		{/if}
	</section>

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
