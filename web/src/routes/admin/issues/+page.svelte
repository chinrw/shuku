<script lang="ts">
	import {
		deleteMissing,
		duplicates,
		issues,
		type DuplicateFile,
		type Issues
	} from '#lib/admin.ts';
	import { bytes, clock, count } from '#lib/format.ts';

	type Tab = 'failed' | 'broken' | 'unreadable' | 'missing' | 'duplicates';

	let data = $state<Issues | null>(null);
	let groups = $state<{ total: number; groups: { files: DuplicateFile[] }[] } | null>(null);
	let tab = $state<Tab>('failed');
	let error = $state<string | null>(null);
	let loading = $state(false);

	let deleting = $state(false);
	let deleteNote = $state('');

	async function remove(ids: number[]) {
		const what = ids.length === 1 ? '这条记录' : `这 ${count(ids.length)} 条记录`;
		if (deleting || !confirm(`删除${what}？阅读进度和分类也一起删除，不能恢复。`)) return;
		deleting = true;
		deleteNote = '';
		try {
			const result = await deleteMissing(ids);
			const kept = result.present.length;
			deleteNote = `删除了 ${count(result.deleted.length)} 条${kept ? `，${count(kept)} 条的文件又出现了，没有删` : ''}。`;
			data = await issues();
		} catch (cause) {
			deleteNote = `没有删除：${cause instanceof Error ? cause.message : String(cause)}`;
		} finally {
			deleting = false;
		}
	}

	async function load() {
		loading = true;
		error = null;
		try {
			data = await issues();
			// 重复文件要全表分组，切到这一栏时才查。
			if (tab === 'duplicates') groups = await duplicates();
		} catch (cause) {
			error = cause instanceof Error ? cause.message : String(cause);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		load();
	});

	function pick(next: Tab) {
		tab = next;
		if (next === 'duplicates' && groups === null) load();
	}

	const hints: Record<Tab, string> = {
		failed: '最近一次扫描读不了的文件和原因。修好文件后在任务页点“立即扫描”重试。',
		broken: '和 LRR 一样照样入库、但有几页读不出来的书（全是 0 的图、0 字节的文件、改了扩展名的别的文件）。坏页在阅读器里显示不出来。',
		unreadable: '没有读权限的子目录。里面已入库的记录不当作丢失。',
		missing:
			'库里有记录、内容目录里已经找不到的文件。改名或移动的文件扫描时会自动认回；确实删掉了的，可以在这里删除记录。',
		duplicates: '完整指纹相同的文件，每组列出全部路径。只统计已经算好指纹的书。'
	};

	let tabs = $derived<{ key: Tab; label: string; total: number | null }[]>([
		{ key: 'failed', label: '扫描失败', total: data?.failed.total ?? null },
		{ key: 'broken', label: '有坏页的书', total: data?.broken.total ?? null },
		{ key: 'unreadable', label: '读不了的目录', total: data?.unreadable.total ?? null },
		{ key: 'missing', label: '文件丢失', total: data?.missing.total ?? null },
		{ key: 'duplicates', label: '重复文件', total: groups?.total ?? null }
	]);

	function shown(listed: { total: number; items: unknown[] }) {
		return listed.total > listed.items.length
			? `只列出前 ${count(listed.items.length)} 条，共 ${count(listed.total)} 条`
			: '';
	}
</script>

<div class="adm-head">
	<h1>问题</h1>
	<span class="adm-sub">
		{#if data?.scanning}来自正在进行的扫描{:else if data?.finished_at}来自 {clock(data.finished_at)} 结束的扫描{:else}服务启动以来还没有扫描过{/if}
	</span>
	<button class="adm-button refresh" disabled={loading} onclick={load}>刷新</button>
</div>

<div class="adm-stack">
	<div class="adm-tabs" role="tablist" aria-label="问题类型">
		{#each tabs as item (item.key)}
			<button
				class="adm-tab"
				role="tab"
				aria-selected={tab === item.key}
				onclick={() => pick(item.key)}
			>
				{item.label}{#if item.total !== null}<span class="count">{count(item.total)}</span>{/if}
			</button>
		{/each}
	</div>
	<p class="adm-note">{hints[tab]}</p>
	{#if error}<p class="error">加载失败：{error}</p>{/if}

	{#if data && tab === 'missing'}
		{#if data.missing.items.length > 0 && !data.scanning}
			<div class="adm-row bulk">
				<button
					class="adm-button danger"
					disabled={deleting}
					onclick={() => remove(data!.missing.items.map((item) => item.id))}
				>
					删除列出的 {count(data.missing.items.length)} 条记录
				</button>
			</div>
		{/if}
		{#if deleteNote}<p class="adm-note" role="status">{deleteNote}</p>{/if}
	{/if}

	{#if data}
		<div class="adm-scroll" role="tabpanel">
			{#if tab === 'failed' || tab === 'unreadable'}
				{@const listed = data[tab]}
				<table class="adm-table">
					<thead><tr><th>{tab === 'failed' ? '路径' : '目录'}</th><th>原因</th></tr></thead>
					<tbody>
						{#each listed.items as item (item.path)}
							<tr><td class="adm-mono">{item.path}</td><td class="error">{item.error}</td></tr>
						{/each}
					</tbody>
				</table>
				{#if listed.items.length === 0}<p class="adm-empty">没有。</p>{/if}
				{#if shown(listed)}<p class="adm-empty">{shown(listed)}</p>{/if}
			{:else if tab === 'broken'}
				<table class="adm-table">
					<thead><tr><th>书</th><th>坏页</th></tr></thead>
					<tbody>
						{#each data.broken.items as item (item.id)}
							<tr>
								<td><a href="/a/{item.id}">{item.title}</a><div class="adm-mono">{item.path}</div></td>
								<td class="adm-mono">{item.pages.join('、')}</td>
							</tr>
						{/each}
					</tbody>
				</table>
				{#if data.broken.items.length === 0}<p class="adm-empty">没有。</p>{/if}
				{#if shown(data.broken)}<p class="adm-empty">{shown(data.broken)}</p>{/if}
			{:else if tab === 'missing'}
				<table class="adm-table">
					<thead><tr><th>标题</th><th>原路径</th><th></th></tr></thead>
					<tbody>
						{#each data.missing.items as item (item.id)}
							<tr>
								<td><a href="/a/{item.id}">{item.title ?? `#${item.id}`}</a></td>
								<td class="adm-mono">{item.path}</td>
								<td class="num">
									<button
										class="adm-button danger small"
										disabled={deleting || data.scanning}
										onclick={() => remove([item.id])}>删除</button
									>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
				{#if data.missing.items.length === 0}<p class="adm-empty">没有丢失的文件。</p>{/if}
				{#if shown(data.missing)}<p class="adm-empty">{shown(data.missing)}</p>{/if}

			{:else if groups}
				<table class="adm-table">
					<thead><tr><th>组</th><th>路径</th><th class="num">大小</th></tr></thead>
					<tbody>
						{#each groups.groups as group, index (group.files[0].id)}
							{#each group.files as file, position (file.id)}
								<tr class:first={position === 0}>
									<td>{position === 0 ? `第 ${index + 1} 组` : ''}</td>
									<td><a class="adm-mono" href="/a/{file.id}">{file.path}</a></td>
									<td class="num">{bytes(file.file_size)}</td>
								</tr>
							{/each}
						{/each}
					</tbody>
				</table>
				{#if groups.groups.length === 0}<p class="adm-empty">没有重复文件。</p>{/if}
				{#if groups.total > groups.groups.length}
					<p class="adm-empty">只列出前 {groups.groups.length} 组，共 {count(groups.total)} 组</p>
				{/if}
			{:else}
				<p class="adm-empty">加载中…</p>
			{/if}
		</div>
	{/if}
</div>

<style>
	.small {
		min-height: 1.9rem;
		padding: 0 0.7rem;
	}

	.bulk {
		justify-content: flex-start;
	}

	.refresh {
		margin-left: auto;
		min-height: 2.1rem;
	}

	tr:not(.first) td {
		border-top-color: transparent;
	}

	a {
		text-decoration: none;
	}

	a:hover {
		color: var(--link);
	}
</style>
