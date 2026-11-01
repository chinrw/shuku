<script lang="ts">
	// “标记读完”按钮，已经读完时显示“已读完”，再点一下取消。书库的详情抽屉和阅读器的
	// 详情面板用，样式由父组件给（和旁边的按钮一致）。
	import { ApiError, type Progress } from '#lib/api.ts';
	import { finishedOf, markFinished } from '#lib/finished.svelte.ts';
	import Icon from '#lib/Icon.svelte';

	let {
		archive,
		progress,
		onchange
	}: {
		archive: number;
		progress: Progress | null | undefined;
		/** 改好后服务端现在的进度，页面自己留着进度时用它更新。 */
		onchange?: (progress: Progress | null) => void;
	} = $props();

	let finished = $derived(finishedOf(archive, progress));
	let busy = $state(false);
	let error = $state('');

	async function toggle() {
		if (busy) return;
		busy = true;
		error = '';
		try {
			// 不能写成 onchange?.(await …)：没有 onchange 时可选调用连参数都不求值。
			const progress = await markFinished(archive, !finished);
			onchange?.(progress);
		} catch (cause) {
			error = cause instanceof ApiError ? cause.message : '网络错误';
		} finally {
			busy = false;
		}
	}
</script>

<button
	class="finished-button"
	class:on={finished}
	aria-pressed={finished}
	title={error ? `没有保存：${error}` : finished ? '读到过最后一页，点一下取消' : '不用读到最后一页也算读完'}
	disabled={busy}
	onclick={toggle}
>
	{#if finished}<Icon name="check" size={14} />已读完{:else}标记读完{/if}
</button>

<style>
	.finished-button {
		display: inline-flex;
		align-items: center;
		gap: 0.25rem;
	}

	.finished-button:disabled {
		opacity: 0.6;
	}
</style>
