<script lang="ts">
	// 详情页和阅读器菜单里把这本书加进或移出静态分类。每次勾选立即提交，失败时恢复那一项。
	import {
		ApiError,
		categories as listCategories,
		createCategory,
		setCategoryMember,
		type Category
	} from '#lib/api.ts';
	import { membersOf, pending, pendingKey, setMember } from '#lib/membership.svelte.ts';

	let {
		archive,
		categories = $bindable(),
		members
	}: {
		archive: number;
		categories: Category[];
		/** 页面加载时这本书所在的静态分类，改过以后以 membership.svelte.ts 里的为准。 */
		members: number[];
	} = $props();

	let current = $derived(membersOf(archive, members));

	let open = $state(false);
	let name = $state('');
	let creating = $state(false);
	let error = $state('');
	let choices = $derived(categories.filter((category) => !category.dynamic));

	// 请求返回前换了书（详情页的相关推荐会复用这一页）时，成员照样按发起时的那本书记，
	// 只是提示和刷新不再落到新书上。`categories` 绑定在父页面上，阅读器关掉菜单销毁这个
	// 组件后，返回的本数照样写回父页面。
	const stale = (target: number) => archive !== target;

	function message(cause: unknown) {
		return cause instanceof ApiError ? cause.message : '网络错误';
	}

	/** 分类的本数来自服务端，增删后重新取一次。失败时保留本地的列表，只是本数不准。 */
	async function reload() {
		categories = await listCategories(fetch).catch(() => categories);
	}

	async function toggle(category: Category) {
		// 同一个分类同时只有一个请求，否则先发的 DELETE 和后发的 PUT 可能倒序提交。
		const target = archive;
		const loaded = members;
		const key = pendingKey(target, category.id);
		if (pending.has(key)) return;
		const member = !membersOf(target, loaded).includes(category.id);
		setMember(target, category.id, member);
		pending.add(key);
		error = '';
		try {
			await setCategoryMember(category.id, target, member);
			if (!stale(target)) await reload();
		} catch (cause) {
			// 只撤销这一项，同时进行的其他勾选可能已经成功。
			setMember(target, category.id, !member);
			if (!stale(target)) error = `没有保存：${message(cause)}`;
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
			if (!current.includes(existing.id)) await toggle(existing);
			return;
		}
		const target = archive;
		creating = true;
		error = '';
		try {
			const { id } = await createCategory(trimmed);
			await setCategoryMember(id, target, true);
			setMember(target, id, true);
			if (stale(target)) return;
			// 先放进本地列表，取列表失败时新分类也在，再输入同名不会重复新建。
			// 等待期间别的勾选触发的刷新可能已经取到了它，按 ID 去重。
			if (!categories.some((category) => category.id === id)) {
				categories = [...categories, {
						id,
						name: trimmed,
						pinned: false,
						dynamic: false,
						count: 1,
						bookmark: false,
						duplicates: false
					}];
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
		加入分类{#if current.length}（{current.length}）{/if}
	</button>
	{#if open}
		<div class="choices" role="group" aria-label="加入分类">
			{#each choices as category (category.id)}
				<label>
					<input
						type="checkbox"
						checked={current.includes(category.id)}
						disabled={pending.has(pendingKey(archive, category.id))}
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
