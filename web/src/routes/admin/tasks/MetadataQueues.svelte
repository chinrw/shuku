<script lang="ts">
	// 任务页的“元数据队列”卡片：每个在线来源排队、候选、完成和失败的本数，暂停时可以继续，可以清空队列。
	import { onMount } from 'svelte';
	import {
		clearMetadataQueue,
		metadataStatus,
		resumeMetadataQueue,
		type MetadataStatus,
		type OnlineProvider
	} from '#lib/admin.ts';
	import { ApiError } from '#lib/api.ts';

	/** 队列在跑时多久刷新一次状态。 */
	const STATUS_INTERVAL_MS = 5000;

	const providers: [OnlineProvider, string][] = [
		['ehentai', 'E-Hentai'],
		['bangumi', 'Bangumi'],
		['anilist', 'AniList']
	];

	let status = $state<MetadataStatus | null>(null);
	/** 最近一次刷新状态失败的原因；成功后清空。失败时仍显示上一次的状态，标明已过期。 */
	let statusError = $state('');
	let error = $state('');

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
		refreshStatus();
		const timer = setInterval(refreshStatus, STATUS_INTERVAL_MS);
		return () => clearInterval(timer);
	});

	async function resume(provider: OnlineProvider) {
		await resumeMetadataQueue(provider).catch((cause) => (error = describe(cause)));
		await refreshStatus();
	}

	async function clearQueue(provider: OnlineProvider, label: string) {
		if (!confirm(`清空 ${label} 的队列？队列里的书不会再去查。`)) return;
		await clearMetadataQueue(provider).catch((cause) => (error = describe(cause)));
		await refreshStatus();
	}
</script>

<section class="adm-card">
	<div class="adm-row"><h2>元数据队列</h2></div>
	{#if statusError}
		<p class="error" role="alert">
			读不到队列状态：{statusError}{status ? '。下面是上一次读到的，可能已经过期' : ''}
		</p>
	{/if}
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#if status}
		{#each providers as [provider, label] (provider)}
			{@const queue = status.providers[provider]}
			<div class="queue">
				<div class="adm-row">
					<strong>{label}</strong>
					<span class="adm-actions">
						{#if queue.paused}
							<button class="adm-button" onclick={() => resume(provider)}>继续</button>
						{/if}
						<button
							class="adm-button danger"
							disabled={queue.queued === 0}
							onclick={() => clearQueue(provider, label)}>清空…</button
						>
					</span>
				</div>
				<span class="adm-label"
					>排队 {queue.queued} 本 · 候选 {queue.candidates} 条 · 本次启动后完成 {queue.done}、失败 {queue.failed}</span
				>
				{#if queue.paused}
					<p class="error">已暂停：{queue.paused}</p>
				{:else if queue.current !== null}
					<p class="adm-note">正在查 <a href="/a/{queue.current}">#{queue.current}</a></p>
				{/if}
				{#if queue.last_error}
					<p class="adm-note">最近一次失败：{queue.last_error}</p>
				{/if}
			</div>
		{/each}
	{/if}
	<p class="adm-note">
		队列里的书不管自动运行开没开都会查。候选是“先放候选”时查到、等确认的结果，用接口或 MCP 读取、改写后批准。来源的开关在设置的<a href="/admin/settings#metadata">“元数据来源”</a>里。
	</p>
</section>

<style>
	.queue {
		display: grid;
		gap: 0.2rem;
		padding-top: 0.5rem;
		border-top: 1px solid var(--border);
	}

	.queue .adm-actions {
		margin-top: 0;
		padding-top: 0;
	}

	.queue p {
		margin: 0;
	}
</style>
