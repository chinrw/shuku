<script lang="ts">
	import { untrack } from 'svelte';
	import { logs, type LogEntry, type LogLevel } from '#lib/admin.ts';
	import { clock } from '#lib/format.ts';
	import { appendLogs } from '#lib/logBuffer.ts';

	const INTERVAL = 3000;
	const levels: { key: LogLevel; label: string }[] = [
		{ key: 'info', label: '全部' },
		{ key: 'warn', label: '警告和错误' },
		{ key: 'error', label: '只看错误' }
	];

	let level = $state<LogLevel>('info');
	let follow = $state(true);
	let entries = $state<LogEntry[]>([]);
	let error = $state<string | null>(null);
	/** 显示中的日志来自哪个服务端进程。 */
	let instance = 0;

	async function fetchNew(current: LogLevel) {
		try {
			let response = await logs(current, entries.at(-1)?.seq ?? 0);
			// 服务端重启过：旧游标比新进程的序号大，会把新日志全部滤掉，从头再取。
			if (instance !== 0 && response.started_at !== instance) {
				response = await logs(current, 0);
				entries = [];
			}
			// 请求期间切换了级别时丢掉这次的结果。
			if (current !== level) return;
			instance = response.started_at;
			entries = appendLogs(entries, response.entries);
			error = null;
		} catch (cause) {
			error = cause instanceof Error ? cause.message : String(cause);
		}
	}

	// 切换级别时从头取；开着自动刷新时之后只取序号更大的新日志。这两个 effect 只依赖
	// level 和 follow：fetchNew 读写 entries，不 untrack 的话每次取到日志都会重新触发。
	$effect(() => {
		const current = level;
		untrack(() => {
			entries = [];
			fetchNew(current);
		});
	});

	$effect(() => {
		if (!follow) return;
		const current = level;
		const timer = setInterval(() => fetchNew(current), INTERVAL);
		return () => clearInterval(timer);
	});

	// 最新的在最上面，不用滚到底。
	let shown = $derived([...entries].reverse());
</script>

<div class="adm-head">
	<h1>日志</h1>
	<span class="adm-sub">进程内最近 1000 条，重启后清空</span>
</div>

<div class="adm-stack">
	<div class="toolbar">
		<div class="adm-tabs" role="group" aria-label="日志级别">
			{#each levels as item (item.key)}
				<button class="adm-tab" aria-pressed={level === item.key} onclick={() => (level = item.key)}>
					{item.label}
				</button>
			{/each}
		</div>
		<label class="follow"><input type="checkbox" bind:checked={follow} />自动刷新</label>
		{#if !follow}
			<button class="adm-button" onclick={() => fetchNew(level)}>取新日志</button>
		{/if}
	</div>
	{#if error}<p class="error">加载失败：{error}</p>{/if}
	<div class="log adm-mono" role="log" aria-live="off">
		{#each shown as entry (entry.seq)}
			<div class="line">
				<span class="time">{clock(entry.time)}</span>
				<span class="level {entry.level}">{entry.level.toUpperCase()}</span>
				<span class="message">{entry.message}</span>
			</div>
		{:else}
			<p class="adm-empty">没有日志。</p>
		{/each}
	</div>
</div>

<style>
	.toolbar {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.6rem 1rem;
	}

	.follow {
		display: inline-flex;
		align-items: center;
		gap: 0.4rem;
		margin-left: auto;
		color: var(--muted);
		font-size: 0.85rem;
	}

	.follow input {
		width: 1.1rem;
		height: 1.1rem;
		accent-color: var(--accent);
	}

	.log {
		padding: 0.5rem 0;
		border-radius: 10px;
		background: var(--sunken);
		line-height: 1.5;
	}

	.line {
		display: grid;
		grid-template-columns: 7.5rem 3.5rem minmax(0, 1fr);
		gap: 0.75rem;
		padding: 0.15rem 1rem;
	}

	.time {
		color: var(--muted);
	}

	.level {
		font-weight: 600;
		color: var(--muted);
	}

	.level.warn {
		color: var(--new);
	}

	.level.error {
		color: var(--danger);
	}

	.message {
		white-space: pre-wrap;
	}

	@media (max-width: 600px) {
		.line {
			grid-template-columns: auto auto;
		}

		.message {
			grid-column: 1 / -1;
		}
	}
</style>
