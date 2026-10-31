<script lang="ts">
	// LRR 的书签：一点把这本书加进或移出书签分类（服务端认出的“收藏”）。没有书签分类时不显示。
	import {
		ApiError,
		categories as listCategories,
		setCategoryMember,
		type Category
	} from '#lib/api.ts';
	import Icon from '#lib/Icon.svelte';
	import { pending, pendingKey } from '#lib/membership.svelte.ts';

	let {
		archive,
		members = $bindable(),
		categories = $bindable(),
		bar = false
	}: {
		archive: number;
		members: number[];
		categories: Category[];
		/** 阅读器工具栏上不要边框，和旁边的图标按钮一样。 */
		bar?: boolean;
	} = $props();

	let bookmark = $derived(categories.find((category) => category.bookmark));
	let marked = $derived(bookmark !== undefined && members.includes(bookmark.id));
	let error = $state('');

	function setMember(id: number, member: boolean) {
		const others = members.filter((other) => other !== id);
		members = member ? [...others, id] : others;
	}

	async function toggle() {
		if (!bookmark) return;
		const target = archive;
		const id = bookmark.id;
		const key = pendingKey(target, id);
		if (pending.has(key)) return;
		const member = !members.includes(id);
		setMember(id, member);
		pending.add(key);
		error = '';
		try {
			await setCategoryMember(id, target, member);
			// 书库上方的“收藏”按钮显示本数，改完重新取一次。取不到时书签已经改好，只是本数旧。
			categories = await listCategories(fetch).catch((cause) => {
				console.warn('刷新分类本数失败', cause);
				return categories;
			});
		} catch (cause) {
			// 请求返回前换了书时，结果不再写进新书的状态。
			if (archive === target) {
				setMember(id, !member);
				error = cause instanceof ApiError ? cause.message : '网络错误';
			}
		} finally {
			pending.delete(key);
		}
	}
</script>

{#if bookmark}
	<button
		class="bookmark"
		class:bar
		class:on={marked}
		aria-pressed={marked}
		aria-label={marked ? '已收藏，点一下取消' : '收藏'}
		title={error ? `没有保存：${error}` : marked ? '已收藏，点一下取消' : '收藏'}
		disabled={pending.has(pendingKey(archive, bookmark.id))}
		onclick={toggle}
	>
		<Icon name="bookmark" filled={marked} />
	</button>
{/if}

<style>
	.bookmark {
		display: inline-flex;
		flex: none;
		align-items: center;
		justify-content: center;
		width: 2.6rem;
		min-height: 2.25rem;
		padding: 0;
		border: 1px solid var(--chip-border, #6d7179);
		border-radius: 8px;
		background: transparent;
		color: inherit;
		cursor: pointer;
	}

	.bookmark.on {
		border-color: var(--accent, #3b97ea);
		color: var(--accent, #3b97ea);
	}

	.bookmark.bar {
		width: 2.25rem;
		border-color: transparent;
	}

	.bookmark.bar.on {
		border-color: transparent;
	}

	.bookmark:disabled {
		opacity: 0.6;
		cursor: progress;
	}
</style>
