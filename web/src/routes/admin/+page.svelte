<script lang="ts">
	import { adminStatus } from '#lib/adminStatus.svelte.ts';
	import { bytes, clock, count, duration, percent } from '#lib/format.ts';
	import ScanProgress from '#lib/ScanProgress.svelte';

	let status = $derived(adminStatus.current);
</script>

<div class="adm-head">
	<h1>概览</h1>
	{#if adminStatus.updated}<span class="adm-sub">每 5 秒刷新 · {clock(adminStatus.updated)}</span>{/if}
</div>

{#if !status}
	<p class="adm-note">加载中…</p>
{:else}
	{@const library = status.library}
	{@const scan = status.scan}
	<div class="adm-stack">
		{#if scan.running}
			<section class="adm-card running" aria-label="扫描">
				<h2>扫描中</h2>
				<ScanProgress {scan} />
				<div><a class="adm-button" href="/admin/tasks">查看任务</a></div>
			</section>
		{/if}

		<div class="adm-grid" style:--min="9rem">
			<div class="adm-card">
				<span class="adm-label">归档</span>
				<span class="adm-figure">{count(library.archives)} 本</span>
				<p class="adm-note">平均每本 {library.archives ? (library.pages / library.archives).toFixed(1) : 0} 页</p>
			</div>
			<div class="adm-card">
				<span class="adm-label">总页数</span>
				<span class="adm-figure">{count(library.pages)}</span>
			</div>
			<div class="adm-card">
				<span class="adm-label">总大小</span>
				<span class="adm-figure">{bytes(library.bytes)}</span>
			</div>
			<div class="adm-card">
				<span class="adm-label">标签</span>
				<span class="adm-figure">{count(library.tags)}</span>
				<p class="adm-note">{library.namespaces} 个命名空间</p>
			</div>
			<div class="adm-card">
				<span class="adm-label">读过</span>
				<span class="adm-figure">{count(library.with_progress)} 本</span>
				<p class="adm-note">最近 7 天 {count(library.read_last_week)} 本</p>
			</div>
		</div>

		<div class="adm-grid" style:--min="16rem">
			<section class="adm-card">
				<h2>完整指纹</h2>
				<div class="adm-row">
					<span class="adm-figure">{count(library.hashed)}</span>
					<span class="adm-label">/ {count(library.archives)} 本</span>
				</div>
				<div class="adm-meter done"><span style:width="{percent(library.hashed, library.archives)}%"></span></div>
				<p class="adm-note">
					{status.hash.running ? '正在补算，逐本顺序读整个文件' : '扫描结束后自动补算，用来认出重复文件'}
				</p>
			</section>
			<section class="adm-card">
				<h2>LRR 导入</h2>
				<dl class="adm-kv">
					<dt>已导入</dt>
					<dd>{count(library.lrr_imported)} 本</dd>
					<dt>暂存待扫描</dt>
					<dd>{count(library.lrr_pending)} 本</dd>
					<dt>分类 / Tankoubon</dt>
					<dd>{library.categories} / {library.tankoubons}</dd>
				</dl>
				{#if library.lrr_pending > 0}
					<div class="adm-meter pending">
						<span style:width="{percent(library.lrr_imported, library.lrr_imported + library.lrr_pending)}%"></span>
					</div>
					<p class="adm-note">暂存的书扫描到时自动套用 LRR 的元数据</p>
				{/if}
			</section>
			<section class="adm-card">
				<h2>存储</h2>
				<dl class="adm-kv">
					<dt>数据库</dt>
					<dd>{bytes(status.storage.database_bytes)}</dd>
					<dt>WAL</dt>
					<dd>{bytes(status.storage.wal_bytes)}</dd>
					<dt>图片缓存</dt>
					<dd>{bytes(status.storage.cache_bytes)} · {count(status.storage.cache_files)} 个文件</dd>
					<dt>快照</dt>
					<dd>
						{clock(status.snapshot.built_at)} 构建，{status.snapshot.read_ms + status.snapshot.build_ms} ms
					</dd>
				</dl>
			</section>
		</div>

		{#if scan.finished_at}
			<section class="adm-card">
				<div class="adm-row">
					<h2>上一次扫描</h2>
					<span class="adm-note">
						{clock(scan.finished_at)} 结束{#if scan.started_at}，用时 {duration(scan.finished_at - scan.started_at)}{/if}
					</span>
					<a href="/admin/issues" class="link">查看问题</a>
				</div>
				{#if scan.error}<p class="error">扫描出错：{scan.error}</p>{/if}
				<div class="counts">
					{#each [['新增', scan.report.added], ['更新', scan.report.updated], ['移动', scan.report.moved], ['未变', scan.report.unchanged], ['跳过', scan.report.skipped], ['失败', scan.report.failed]] as [label, value] (label)}
						<div class:bad={label === '失败' && Number(value) > 0}>
							<span class="adm-label">{label}</span>
							<strong>{count(Number(value))}</strong>
						</div>
					{/each}
				</div>
			</section>
		{/if}
	</div>
{/if}

<style>
	.link {
		color: var(--link);
		font-size: 0.85rem;
		text-decoration: none;
	}

	.counts {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(6.5rem, 1fr));
		gap: 0.6rem;
	}

	.counts div {
		display: flex;
		flex-direction: column;
		gap: 0.2rem;
		padding: 0.6rem 0.75rem;
		border-radius: 8px;
		background: var(--bg);
	}

	.counts strong {
		font-size: 1.1rem;
		font-variant-numeric: tabular-nums;
	}

	.counts .bad strong {
		color: var(--danger);
	}
</style>
