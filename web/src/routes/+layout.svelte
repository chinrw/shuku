<script lang="ts">
	import '../app.css';
	import { onMount } from 'svelte';
	import { dev } from '$app/env';
	import { invalidateAll } from '$app/navigation';

	let { children } = $props();

	// 浏览器只在 HTTPS 和 localhost 下提供 Service Worker，局域网 HTTP 下这里什么都不做。
	// 开发时不注册：开发服务器没有构建出的文件清单，缓存的接口结果还会让改动看起来没生效。
	onMount(() => {
		if (dev || !('serviceWorker' in navigator)) return;
		// 第一次打开时页面的数据在 Service Worker 接管之前就取完了，没进缓存。接管后重新取一次，
		// 这样第一次打开的书断网后也能重开。
		if (!navigator.serviceWorker.controller) {
			navigator.serviceWorker.addEventListener('controllerchange', () => invalidateAll(), {
				once: true
			});
		}
		navigator.serviceWorker
			.register('/service-worker.js', { type: 'module' })
			.catch((error) => console.warn('Service Worker 注册失败', error));
	});
</script>

{@render children()}
