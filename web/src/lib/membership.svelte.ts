import { SvelteMap, SvelteSet } from 'svelte/reactivity';

// 正在提交的“归档:分类”，分类勾选和书签按钮共用。放在模块里而不是组件里：阅读器关掉菜单会
// 销毁组件，重新打开时也要知道上一次的请求还没回来，同一本书的同一个分类不能同时有两个请求，
// 否则先发的 DELETE 和后发的 PUT 可能倒序提交。
export const pending = new SvelteSet<string>();

export function pendingKey(archive: number, category: number): string {
	return `${archive}:${category}`;
}

// 这次打开网页以来在这里加进或移出的“归档:分类”。书库列表、详情抽屉、阅读器里的书签按钮
// 和分类勾选都读写这一份，在一处改了别处跟着变；请求失败的回滚在组件销毁后也照样写回这里。
// 只记改过的那几对，其余分类仍以各页面最新加载的数据为准。
const changed = new SvelteMap<string, boolean>();

/** 这本书现在所在的静态分类：页面加载的 `loaded` 加上这里改过的。 */
export function membersOf(archive: number, loaded: number[]): number[] {
	let members = loaded;
	for (const [key, member] of changed) {
		const [owner, category] = key.split(':').map(Number);
		if (owner !== archive) continue;
		const others = members.filter((other) => other !== category);
		members = member ? [...others, category] : others;
	}
	return members;
}

/** 记下把这本书加进或移出一个分类。 */
export function setMember(archive: number, category: number, member: boolean) {
	changed.set(pendingKey(archive, category), member);
}

/** 批量改了分类成员以后，以服务端重新加载的数据为准。 */
export function forgetMembers() {
	changed.clear();
}
