<script module lang="ts">
	import { SvelteSet } from 'svelte/reactivity';

	// 正在提交的“归档:分类”。放在模块里而不是组件里：阅读器关掉菜单会销毁组件，
	// 重新打开时也要知道上一次的请求还没回来，同一本书的同一个分类不能同时有两个请求。
	const pending = new SvelteSet<string>();
</script>

<script lang="ts">
	// 详情页和阅读器菜单里把这本书加进或移出静态分类。每次勾选立即提交，失败时恢复那一项。
	import {
		ApiError,
		categories as listCategories,
		createCategory,
		setCategoryMember,
		type Category
	} from '#lib/api.ts';

	let {
		archive,
		categories = $bindable(),
		members = $bindable()
	}: { archive: number; categories: Category[]; members: number[] } = $props();

	let open = $state(false);
	let name = $state('');
	let creating = $state(false);
	let error = $state('');
	let choices = $derived(categories.filter((category) => !category.dynamic));

	// 请求返回前换了书（详情页的相关推荐会复用这一页）时，结果只落到发起时的那本书上，
	// 不再改界面，免得把旧书的结果写进新书的勾选状态。`members` 和 `categories` 绑定在
	// 父页面上，阅读器关掉菜单销毁这个组件后，返回的结果照样写回父页面。
	const stale = (target: number) => archive !== target;

	function message(cause: unknown) {
		return cause instanceof ApiError ? cause.message : '网络错误';
	}

	/** 分类的本数来自服务端，增删后重新取一次。失败时保留本地的列表，只是本数不准。 */
	async function reload() {
		categories = await listCategories(fetch).catch(() => categories);
	}

	function setMember(id: number, member: boolean) {
		const others = members.filter((other) => other !== id);
		members = member ? [...others, id] : others;
	}

	async function toggle(category: Category) {
		// 同一个分类同时只有一个请求，否则先发的 DELETE 和后发的 PUT 可能倒序提交。
		const target = archive;
		const key = `${target}:${category.id}`;
		if (pending.has(key)) return;
		const member = !members.includes(category.id);
		setMember(category.id, member);
		pending.add(key);
		error = '';
		try {
			await setCategoryMember(category.id, target, member);
			if (!stale(target)) await reload();
		} catch (cause) {
			// 只撤销这一项，同时进行的其他勾选可能已经成功。
			if (!stale(target)) {
				setMember(category.id, !member);
				error = `没有保存：${message(cause)}`;
			}
		} finally {
			pending.delete(key);
		}
	}

	async function create(event: SubmitEvent) {
		event.preventDefault();
		const trimmed = name.trim();
		if (!trimmed || creating) return;
		// 同名的静态分类已经有了，就直接加进去，不再建一个重名的。
		const existing = choices.find((category) => category.name === trimmed);
		if (existing) {
			name = '';
			if (!members.includes(existing.id)) await toggle(existing);
			return;
		}
		const target = archive;
		creating = true;
		error = '';
		try {
			const { id } = await createCategory(trimmed);
			await setCategoryMember(id, target, true);
			if (stale(target)) return;
			setMember(id, true);
			// 先放进本地列表，取列表失败时新分类也在，再输入同名不会重复新建。
			// 等待期间别的勾选触发的刷新可能已经取到了它，按 ID 去重。
			if (!categories.some((category) => category.id === id)) {
				categories = [...categories, { id, name: trimmed, pinned: false, dynamic: false, count: 1 }];
			}
			name = '';
			await reload();
		} catch (cause) {
			if (stale(target)) return;
			error = `没有新建：${message(cause)}`;
			await reload();
		} finally {
			creating = false;
		}
	}
</script>

<div class="picker">
	<button class="toggle" aria-expanded={open} onclick={() => (open = !open)}>
		加入分类{#if members.length}（{members.length}）{/if}
	</button>
	{#if open}
		<div class="choices" role="group" aria-label="加入分类">
			{#each choices as category (category.id)}
				<label>
					<input
						type="checkbox"
						checked={members.includes(category.id)}
						disabled={pending.has(`${archive}:${category.id}`)}
						onchange={() => toggle(category)}
					/>
					<span class="name">{category.name}</span>
					<span class="count">{category.count}</span>
				</label>
			{:else}
				<p class="empty">还没有分类，在下面新建一个。</p>
			{/each}
			<form onsubmit={create}>
				<input bind:value={name} placeholder="新分类名" maxlength="100" aria-label="新分类名" />
				<button disabled={!name.trim() || creating}>新建并加入</button>
			</form>
			{#if error}<p class="error" role="alert">{error}</p>{/if}
		</div>
	{/if}
</div>

<style>
	.picker {
		display: grid;
		gap: 0.5rem;
	}

	.toggle {
		justify-self: start;
		padding: 0.5rem 0.9rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: var(--surface);
		color: inherit;
		font: inherit;
		cursor: pointer;
	}

	.toggle[aria-expanded='true'] {
		border-color: var(--accent);
	}

	.choices {
		display: grid;
		gap: 0.1rem;
		max-width: min(24rem, 100%);
		padding: 0.5rem;
		border: 1px solid var(--border);
		border-radius: 8px;
		background: var(--surface);
	}

	label {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		min-height: 2.25rem;
		padding: 0 0.4rem;
		border-radius: 5px;
		cursor: pointer;
	}

	label:hover {
		background: var(--raised);
	}

	.name {
		flex: 1;
		overflow-wrap: anywhere;
	}

	.count,
	.empty {
		color: var(--muted);
		font-size: 0.85rem;
	}

	.empty {
		margin: 0.25rem 0.4rem;
	}

	form {
		display: flex;
		gap: 0.4rem;
		margin-top: 0.4rem;
	}

	form input {
		flex: 1;
		min-width: 0;
		padding: 0.45rem 0.6rem;
		border: 1px solid var(--border);
		border-radius: 5px;
		background: var(--bg);
		color: inherit;
		font: inherit;
	}

	form button {
		padding: 0.45rem 0.7rem;
		border: 1px solid var(--accent);
		border-radius: 5px;
		background: var(--accent);
		color: white;
		font: inherit;
		cursor: pointer;
	}

	form button:disabled {
		opacity: 0.5;
		cursor: default;
	}

	.error {
		margin: 0.25rem 0.4rem;
		color: var(--danger);
		font-size: 0.85rem;
	}
</style>
