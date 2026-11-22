<script lang="ts">
	// 详情页的“删除”：连原文件一起删除，要在确认框里勾选一次才能点。书库目录只读时
	// 服务端报 `deletable: false`，确认框只说明原因，挂成读写后不用改什么就能删。
	import { ApiError, deleteArchive, type ArchiveDetail } from '#lib/api.ts';
	import { bytes } from '#lib/format.ts';

	let { book, ondeleted }: { book: ArchiveDetail; ondeleted: () => Promise<void> } = $props();

	let dialog = $state<HTMLDialogElement>();
	let understood = $state(false);
	let deleting = $state(false);
	let error = $state('');

	let paths = $derived(book.files ?? [{ path: book.path, deletable: book.deletable }]);
	let multiple = $derived(paths.length > 1);
	let selected = $state('');
	let canDelete = $derived(paths.find((file) => file.path === selected)?.deletable);

	function open() {
		selected = book.path;
		understood = false;
		error = '';
		dialog?.showModal();
	}

	function close() {
		if (!deleting) dialog?.close();
	}

	async function confirm(event: SubmitEvent) {
		event.preventDefault();
		if (deleting || !understood || !canDelete) return;
		// 返回前详情页可能已经换成另一本书，组件和回调跟着换了，所以先固定这一本的回调。
		const done = ondeleted;
		deleting = true;
		error = '';
		try {
			await deleteArchive(book.id, { file: true, path: multiple ? selected : undefined });
			dialog?.close();
			await done();
		} catch (cause) {
			error = cause instanceof ApiError ? cause.message : '网络错误';
		} finally {
			deleting = false;
		}
	}
</script>

<button class="trigger" onclick={open}>删除</button>

<!-- 点遮罩不关，免得误触；按 Esc 或“取消”关闭。 -->
<dialog bind:this={dialog} oncancel={(event) => deleting && event.preventDefault()}>
	<form onsubmit={confirm}>
		<h2>{multiple ? '删除一份文件' : '删除这本书'}</h2>
		<p class="title">{book.title}</p>
		{#if multiple}
			<label>要删除的文件
				<select bind:value={selected} disabled={deleting} onchange={() => (understood = false)}>
					{#each paths as file (file.path)}<option value={file.path}>{file.path}</option>{/each}
				</select>
			</label>
		{:else}
			<p class="path">{book.path} · {bytes(book.file_size)}</p>
		{/if}
		{#if canDelete}
			<p>
				{#if multiple}只删除选中的文件，保留其他路径、阅读进度、分类和标签。
				{:else}原文件会从书库目录里删除（散放图片的文件夹删的是其中的图片），阅读进度、分类和标签也一起删除，不能恢复。{/if}
			</p>
			<label>
				<input type="checkbox" bind:checked={understood} disabled={deleting} />
				我确认要永久删除这个文件
			</label>
		{:else}
			<p class="readonly" role="status">
				文件不在原处，或服务没有写入所在目录的权限，不能删除这个文件。
			</p>
		{/if}
		{#if error}<p class="error" role="alert">没有删除：{error}</p>{/if}
		<div class="buttons">
			{#if canDelete}
				<button class="danger" disabled={!understood || deleting}>
					{deleting ? '正在删除…' : '删除'}
				</button>
			{/if}
			<button type="button" onclick={close} disabled={deleting}>
				{canDelete ? '取消' : '关闭'}
			</button>
		</div>
	</form>
</dialog>

<style>
	button {
		padding: 0.5rem 0.9rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: var(--surface);
		color: inherit;
		font: inherit;
		cursor: pointer;
	}

	button:disabled {
		opacity: 0.6;
		cursor: default;
	}

	.trigger {
		color: var(--danger);
	}

	dialog {
		width: min(28rem, calc(100% - 2rem));
		box-sizing: border-box;
		padding: 1.1rem 1.2rem;
		border: 1px solid var(--danger);
		border-radius: 10px;
		background: var(--raised);
		color: inherit;
	}

	dialog::backdrop {
		background: rgb(0 0 0 / 0.55);
	}

	form {
		display: grid;
		gap: 0.6rem;
		line-height: 1.6;
	}

	h2 {
		margin: 0;
		font-size: 1.05rem;
		color: var(--danger);
	}

	p {
		margin: 0;
		font-size: 0.9rem;
	}

	.title {
		font-weight: 600;
		overflow-wrap: anywhere;
	}

	.path {
		color: var(--muted);
		font-family: ui-monospace, monospace;
		font-size: 0.8rem;
		overflow-wrap: anywhere;
	}

	.readonly {
		color: var(--muted);
	}

	label {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		font-size: 0.9rem;
		cursor: pointer;
	}

	.buttons {
		display: flex;
		justify-content: flex-end;
		gap: 0.5rem;
		margin-top: 0.3rem;
	}

	.danger {
		border-color: var(--danger);
		background: var(--danger);
		color: var(--on-accent);
	}
</style>
