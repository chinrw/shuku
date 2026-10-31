<script lang="ts">
	// 详情页里对这本书手动运行一个元数据功能。和 LRR 的插件一样只加标签，标题按控制台的设置决定
	// 换不换。写入后重新载入这一页，标签表和标题马上是新的。
	import { invalidateAll } from '$app/navigation';
	import { ApiError, fetchMetadata, type FoundMetadata, type MetadataProvider } from '#lib/api.ts';

	let { archive }: { archive: number } = $props();

	let open = $state(false);
	let url = $state('');
	let running = $state<MetadataProvider | null>(null);
	let result = $state('');
	let error = $state('');

	const providers: [MetadataProvider, string][] = [
		['filename', '文件名'],
		['comicinfo', 'ComicInfo'],
		['ehentai', 'E-Hentai']
	];

	function describe(found: FoundMetadata) {
		const parts = [`加了 ${found.tags.length} 个标签`];
		if (found.title) parts.push(`标题改为“${found.title}”`);
		if (found.source_url) parts.push('记下了来源');
		return parts.join('，');
	}

	async function run(provider: MetadataProvider) {
		if (running) return;
		const target = archive;
		running = provider;
		result = '';
		error = '';
		try {
			const found = await fetchMetadata(target, provider, provider === 'ehentai' ? url.trim() : undefined);
			if (target !== archive) return;
			result = describe(found);
			await invalidateAll();
		} catch (cause) {
			if (target === archive) error = cause instanceof ApiError ? cause.message : '网络错误';
		} finally {
			running = null;
		}
	}
</script>

<div class="fetcher">
	<button class="toggle" aria-expanded={open} onclick={() => (open = !open)}>获取元数据</button>
	{#if open}
		<div class="panel">
			<div class="buttons">
				{#each providers as [provider, label] (provider)}
					<button disabled={running !== null} onclick={() => run(provider)}>
						{running === provider ? `${label}…` : label}
					</button>
				{/each}
			</div>
			<label>
				<span>E-Hentai 画廊地址（可选，填了就不搜索）</span>
				<input
					type="url"
					bind:value={url}
					placeholder="https://exhentai.org/g/…/…/"
					autocomplete="off"
				/>
			</label>
			{#if result}<p class="result" role="status">{result}</p>{/if}
			{#if error}<p class="error" role="alert">{error}</p>{/if}
		</div>
	{/if}
</div>

<style>
	.toggle,
	.buttons button {
		padding: 0.4rem 0.8rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: var(--surface);
		color: inherit;
		font: inherit;
		font-size: 0.85rem;
	}

	.panel {
		display: grid;
		gap: 0.6rem;
		margin-top: 0.6rem;
		padding: 0.75rem;
		border-radius: 9px;
		background: var(--surface);
	}

	.buttons {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
	}

	.buttons button {
		background: var(--raised);
	}

	label {
		display: grid;
		gap: 0.3rem;
		color: var(--muted);
		font-size: 0.78rem;
	}

	input {
		box-sizing: border-box;
		width: 100%;
		padding: 0.45rem 0.6rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: var(--sunken);
		color: var(--fg);
		font: inherit;
		font-size: 0.85rem;
	}

	.result,
	.error {
		margin: 0;
		font-size: 0.82rem;
	}

	.error {
		color: var(--danger);
	}
</style>
