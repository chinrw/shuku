<script lang="ts">
	import type { ScanTask } from './admin';
	import { clock, count, duration, percent } from './format';

	// 扫描进度条和预计剩余时间，概览页和任务页共用。
	let { scan }: { scan: ScanTask } = $props();

	let done = $derived(percent(scan.processed, scan.total));
	let remaining = $derived.by(() => {
		if (!scan.started_at || scan.processed === 0 || scan.total === 0) return null;
		const elapsed = Date.now() - scan.started_at;
		return (elapsed / scan.processed) * (scan.total - scan.processed);
	});
</script>

<div class="adm-row">
	<span>
		{#if scan.total > 0}已处理 {count(scan.processed)} / {count(scan.total)} 个文件{:else}正在列出内容目录…{/if}
	</span>
	<span class="adm-label">{done}%</span>
</div>
<div class="adm-meter" role="progressbar" aria-valuenow={done} aria-valuemin={0} aria-valuemax={100}>
	<span style:width="{done}%"></span>
</div>
<p class="adm-note">
	{#if scan.started_at}{clock(scan.started_at)} 开始{/if}{#if remaining !== null}
		· 预计还要 {duration(remaining)}{/if}
</p>
