<script lang="ts">
	import '../app.css';
	import { onMount } from 'svelte';
	import { dev } from '$app/env';
	import { settings } from '#lib/settings.svelte.ts';
	import { activeThemes, applyTheme } from '#lib/themes.ts';
	import { warmUp } from '#lib/warmup.ts';

	let { children } = $props();

	// 配色和页面在同一轮更新里套上，第一次打开（还没有 app.html 用的缓存）时也不会先画出没有颜色的页面。
	// 跟随系统时，系统切换深浅色马上换配色。
	const prefs = settings();
	const lightQuery = window.matchMedia('(prefers-color-scheme: light)');
	let systemLight = $state(lightQuery.matches);
	let pair = $derived(activeThemes(prefs.theme, prefs.darkTheme, prefs.lightTheme));
	let theme = $derived(systemLight ? pair.light : pair.dark);
	$effect.pre(() => applyTheme(theme, pair));
	$effect(() => {
		const onChange = (event: MediaQueryListEvent) => (systemLight = event.matches);
		lightQuery.addEventListener('change', onChange);
		return () => lightQuery.removeEventListener('change', onChange);
	});

	// 浏览器只在 HTTPS 和 localhost 下提供 Service Worker，局域网 HTTP 下这里什么都不做。
	// 开发时不注册：开发服务器没有构建出的文件清单，缓存的接口结果还会让改动看起来没生效。
	onMount(() => {
		if (dev || !('serviceWorker' in navigator)) return;
		// 第一次打开时取过的接口没进 Service Worker 的缓存，接管时补取（见 warmup.ts）。不用 refreshAll
		// 或 invalidateAll 重跑 load：它们会改写 page.state，开着的阅读器面板、书库详情抽屉会自己关上。
		if (!navigator.serviceWorker.controller) {
			navigator.serviceWorker.addEventListener('controllerchange', () => warmUp(location.origin), {
				once: true
			});
		}
		navigator.serviceWorker
			.register('/service-worker.js', { type: 'module' })
			.catch((error) => console.warn('Service Worker 注册失败', error));
	});
</script>

{@render children()}
