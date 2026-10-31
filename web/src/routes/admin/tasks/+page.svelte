<script lang="ts">
	import { clearCache, refreshSnapshot, startHash, startScan } from '#lib/admin.ts';
	import { adminStatus } from '#lib/adminStatus.svelte.ts';
	import { bytes, clock, count, duration, percent } from '#lib/format.ts';
	import Icon from '#lib/Icon.svelte';
	import ScanProgress from '#lib/ScanProgress.svelte';

	let status = $derived(adminStatus.current);
	let pending = $state<string | null>(null);
	let message = $state<{ text: string; error: boolean } | null>(null);

	// 操作期间按钮禁用；完成后立即刷新状态，不等下一轮轮询。
	async function run(name: string, action: () => Promise<string>) {
		pending = name;
		message = null;
		try {
			message = { text: await action(), error: false };
		} catch (error) {
			message = { text: error instanceof Error ? error.message : String(error), error: true };
		} finally {
			pending = null;
			await adminStatus.refresh();
		}
	}

	const scan = () =>
		run('scan', async () => {
			await startScan();
			return '已开始扫描，结束后会接着补算指纹';
		});
	const hash = () =>
		run('hash', async () => {
			await startHash();
			return '已开始补算指纹';
		});
	const rebuild = () =>
		run('refresh', async () => {
			await refreshSnapshot();
			return '快照已重建';
		});
	function clear() {
		if (!confirm('清空图片缓存？缩略图和缩放后的图片会在下次打开时重新生成。')) return;
		run('cache', async () => {
			const removed = await clearCache();
			return `删除了 ${count(removed.removed_files)} 个文件，${bytes(removed.removed_bytes)}`;
		});
	}

	function stateLabel(task: { running: boolean; error: string | null }) {
		return task.running ? '运行中' : task.error ? '出错' : '空闲';
	}
</script>

<div class="adm-head">
	<h1>任务</h1>
	<span class="adm-sub">同一种任务同时只跑一个</span>
</div>

{#if message}
	<p class="message" class:error={message.error} role="status">{message.text}</p>
{/if}

{#if !status}
	<p class="adm-note">加载中…</p>
{:else}
	{@const scanTask = status.scan}
	{@const hashTask = status.hash}
	<div class="adm-grid" style:--min="20rem">
		<section class="adm-card" class:running={scanTask.running}>
			<div class="adm-row">
				<h2>扫描内容目录</h2>
				<span class="adm-state" class:running={scanTask.running} class:failed={!!scanTask.error}>
					{stateLabel(scanTask)}
				</span>
			</div>
			{#if scanTask.running}
				<ScanProgress scan={scanTask} />
			{:else if scanTask.finished_at}
				<p class="adm-note">
					上次 {clock(scanTask.finished_at)} 结束{#if scanTask.started_at}，用时 {duration(scanTask.finished_at - scanTask.started_at)}{/if}
				</p>
			{:else}
				<p class="adm-note">服务启动以来还没有扫描过</p>
			{/if}
			{#if scanTask.error}<p class="error">{scanTask.error}</p>{/if}
			<dl class="adm-kv left counts">
				<dt>新增</dt><dd>{count(scanTask.report.added)}</dd>
				<dt>更新</dt><dd>{count(scanTask.report.updated)}</dd>
				<dt>移动</dt><dd>{count(scanTask.report.moved)}</dd>
				<dt>未变</dt><dd>{count(scanTask.report.unchanged)}</dd>
				<dt>其他文件</dt><dd>{count(scanTask.report.skipped)}</dd>
				<dt>失败</dt><dd class:error={scanTask.report.failed > 0}>{count(scanTask.report.failed)}</dd>
			</dl>
			<div class="adm-actions">
				<button
					class="adm-button primary"
					disabled={scanTask.running || pending !== null}
					onclick={scan}
				>
					{scanTask.running ? '扫描中…' : '立即扫描'}
				</button>
			</div>
		</section>

		<section class="adm-card" class:running={hashTask.running}>
			<div class="adm-row">
				<h2>补算完整指纹</h2>
				<span class="adm-state" class:running={hashTask.running} class:failed={!!hashTask.error}>
					{stateLabel(hashTask)}
				</span>
			</div>
			<div class="adm-row">
				<span>{count(status.library.hashed)} / {count(status.library.archives)} 本</span>
				<span class="adm-label">{percent(status.library.hashed, status.library.archives)}%</span>
			</div>
			<div class="adm-meter done">
				<span style:width="{percent(status.library.hashed, status.library.archives)}%"></span>
			</div>
			<p class="adm-note">
				{#if hashTask.running}本轮已算 {count(hashTask.report.hashed)} / {count(hashTask.total)} 本{:else if hashTask.finished_at}上次 {clock(hashTask.finished_at)} 结束，算了 {count(hashTask.report.hashed)} 本，跳过 {count(hashTask.report.skipped)}，失败 {count(hashTask.report.failed)}{:else}扫描结束后自动开始{/if}
			</p>
			{#if hashTask.error}<p class="error">{hashTask.error}</p>{/if}
			<div class="adm-actions">
				<button class="adm-button" disabled={hashTask.running || pending !== null} onclick={hash}>
					{hashTask.running ? '补算中…' : '现在补算'}
				</button>
			</div>
		</section>

		<section class="adm-card">
			<div class="adm-row"><h2>搜索快照</h2></div>
			<dl class="adm-kv left">
				<dt>构建于</dt><dd>{clock(status.snapshot.built_at)}</dd>
				<dt>耗时</dt><dd>读库 {status.snapshot.read_ms} ms，建索引 {status.snapshot.build_ms} ms</dd>
				<dt>本数</dt><dd>{count(status.snapshot.archives)}</dd>
			</dl>
			<p class="adm-note">写入后约 10 秒自动刷新，一般不用手动重建。用迁移工具在服务运行时导入后可以点这里。</p>
			<div class="adm-actions">
				<button class="adm-button" disabled={pending !== null} onclick={rebuild}>
					<Icon name="refresh" size={16} />重建快照
				</button>
			</div>
		</section>

		<section class="adm-card">
			<div class="adm-row"><h2>图片缓存</h2></div>
			<dl class="adm-kv left">
				<dt>缩略图和缩放图</dt>
				<dd>{bytes(status.storage.cache_bytes)} · {count(status.storage.cache_files)} 个文件</dd>
			</dl>
			<p class="adm-note">大小每分钟统计一次。清空后图片按需重新生成，第一次打开会慢一些。</p>
			<div class="adm-actions">
				<button class="adm-button danger" disabled={pending !== null} onclick={clear}>清空缓存…</button>
			</div>
		</section>
	</div>
{/if}

<style>
	.message {
		margin: 0 0 1rem;
		padding: 0.6rem 0.9rem;
		border-radius: 8px;
		background: var(--surface);
	}

	.counts {
		grid-template-columns: repeat(3, max-content minmax(0, 1fr));
	}
</style>
