import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
	plugins: [
		sveltekit({
			// 不预渲染任何页面，所有路径都由服务端回退到 index.html，再交给客户端路由。
			adapter: adapter({ fallback: 'index.html' })
		})
	],
	server: {
		// 开发时把 API 请求转给本机的 shuku 服务。
		proxy: { '/api': 'http://localhost:3001' }
	},
	test: { include: ['src/**/*.test.ts'] }
});
