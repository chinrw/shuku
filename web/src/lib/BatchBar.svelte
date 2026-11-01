<script lang="ts">
	// 书库页的批量操作栏：给选中的书或整个搜索结果加减标签、加进或移出静态分类。
	import { ApiError, batch, type BatchSelection, type Category } from '#lib/api.ts';

	let {
		selection,
		count,
		total,
		all = $bindable(),
		categories,
		ondone,
		oncancel
	}: {
		selection: BatchSelection;
		/** 选中的本数；`all` 时是搜索结果的总数。 */
		count: number;
		total: number;
		all: boolean;
		categories: Category[];
		/** `tags` 表示改了标签，搜索结果要等服务端的快照重建才看得到。 */
		ondone: (message: string, tags: boolean) => void;
		oncancel: () => void;
	} = $props();

	let addTags = $state('');
	let removeTags = $state('');
	let addTo = $state('');
	let removeFrom = $state('');
	let busy = $state(false);
	let error = $state('');
	let choices = $derived(categories.filter((category) => !category.dynamic));

	const split = (text: string) =>
		text
			.split(',')
			.map((tag) => tag.trim())
			.filter(Boolean);

	let ready = $derived(
		count > 0 && (split(addTags).length > 0 || split(removeTags).length > 0 || !!addTo || !!removeFrom)
	);

	async function run(event: SubmitEvent) {
		event.preventDefault();
		if (!ready || busy) return;
		busy = true;
		error = '';
		// 请求期间输入框还能改，提交的内容和“改了标签没有”都在发出前定下来。
		const change = {
			add_tags: split(addTags),
			remove_tags: split(removeTags),
			add_to_category: addTo ? Number(addTo) : undefined,
			remove_from_category: removeFrom ? Number(removeFrom) : undefined
		};
		const tags = change.add_tags.length > 0 || change.remove_tags.length > 0;
		try {
			const changed = await batch(selection, change);
			ondone(`改了 ${changed} 本${tags ? '，标签约 10 秒后出现在搜索结果里' : ''}`, tags);
		} catch (cause) {
			error = cause instanceof ApiError ? cause.message : '网络错误';
		} finally {
			busy = false;
		}
	}
</script>

<form class="bar" onsubmit={run} aria-label="批量操作">
	<div class="line">
		<strong>已选 {count} 本</strong>
		<label class="all">
			<input type="checkbox" bind:checked={all} />
			全部 {total} 本搜索结果
		</label>
		<button type="button" class="plain" onclick={oncancel}>取消</button>
	</div>
	<div class="line fields">
		<input bind:value={addTags} placeholder="加标签，逗号分隔" aria-label="加的标签" />
		<input bind:value={removeTags} placeholder="去掉标签，逗号分隔" aria-label="去掉的标签" />
		<select bind:value={addTo} aria-label="加入分类">
			<option value="">加入分类…</option>
			{#each choices as category (category.id)}
				<option value={String(category.id)}>{category.name}</option>
			{/each}
		</select>
		<select bind:value={removeFrom} aria-label="移出分类">
			<option value="">移出分类…</option>
			{#each choices as category (category.id)}
				<option value={String(category.id)}>{category.name}</option>
			{/each}
		</select>
		<button class="primary" disabled={!ready || busy}>{busy ? '正在改…' : '执行'}</button>
	</div>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
</form>

<style>
	.bar {
		position: fixed;
		inset: auto 0 0;
		z-index: 20;
		display: grid;
		gap: 0.5rem;
		padding: 0.6rem 1rem;
		padding-bottom: max(0.6rem, env(safe-area-inset-bottom));
		border-top: 1px solid var(--border);
		background: var(--surface);
		box-shadow: 0 -4px 16px rgb(0 0 0 / 0.25);
	}

	.line {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.5rem 0.75rem;
	}

	/* 电脑上和书库内容（窗口的 90%）对齐。 */
	@media (min-width: 1024px) {
		.bar {
			padding-inline: calc(5% + 1.25rem);
		}
	}

	.all {
		display: inline-flex;
		align-items: center;
		gap: 0.35rem;
		color: var(--muted);
		font-size: 0.9rem;
	}

	.plain {
		margin-left: auto;
		border: none;
		background: none;
		color: var(--link);
		font: inherit;
		cursor: pointer;
	}

	.fields input {
		flex: 1 1 10rem;
		min-width: 0;
	}

	.fields input,
	.fields select {
		height: 2.25rem;
		padding: 0 0.6rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: var(--bg);
		color: inherit;
		font: inherit;
	}

	.primary {
		height: 2.25rem;
		padding: 0 1rem;
		border: 1px solid var(--accent);
		border-radius: 6px;
		background: var(--accent);
		color: white;
		font: inherit;
		cursor: pointer;
	}

	.primary:disabled {
		opacity: 0.5;
		cursor: default;
	}

	.error {
		margin: 0;
		color: var(--danger);
		font-size: 0.85rem;
		text-align: center;
	}
</style>
