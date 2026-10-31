import { SvelteSet } from 'svelte/reactivity';

// 正在提交的“归档:分类”，分类勾选和书签按钮共用。放在模块里而不是组件里：阅读器关掉菜单会
// 销毁组件，重新打开时也要知道上一次的请求还没回来，同一本书的同一个分类不能同时有两个请求，
// 否则先发的 DELETE 和后发的 PUT 可能倒序提交。
export const pending = new SvelteSet<string>();

export function pendingKey(archive: number, category: number): string {
	return `${archive}:${category}`;
}
