import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
	plugins: [
		sveltekit({
			// 不预渲染任何页面，所有路径都由服务端回退到 index.html，再交给客户端路由。
			adapter: adapter({ fallback: 'index.html' }),
			// 由根布局注册，开发时不注册，见 src/routes/+layout.svelte。
			serviceWorker: { register: false }
		})
	],
	server: {
		// 开发时把 API 请求转给本机的 shuku 服务。
		proxy: { '/api': 'http://localhost:3002' }
	},
	test: { include: ['src/**/*.test.ts'] }
});
