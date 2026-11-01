import { expect, test } from 'vitest';

// invalidateAll 会把 page.state 清空（这里只认调用、导入和 goto 的选项，不认注释）：阅读器面板和书库的详情抽屉靠它和历史记录同步，
// 清空后面板自己关上，历史里却还留着那条记录。重新加载数据一律用 refreshAll。
test('the web client never calls invalidateAll', () => {
	const sources = import.meta.glob('/src/**/*.{svelte,ts}', {
		query: '?raw',
		import: 'default',
		eager: true
	});
	const offenders = Object.entries(sources)
		.filter(([path]) => !path.endsWith('.test.ts'))
		.filter(([, source]) => /\binvalidateAll\s*[(,}:]/.test(source as string))
		.map(([path]) => path);
	expect(Object.keys(sources).length).toBeGreaterThan(10);
	expect(offenders).toEqual([]);
});
