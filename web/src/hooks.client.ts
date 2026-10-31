import type { HandleClientError } from '@sveltejs/kit/hooks';
import { ApiError } from '#lib/api.ts';

// 页面加载出错时 SvelteKit 默认只显示“Internal Error”。离线打开没缓存过的书时，fetch 连不上
// 服务端会抛 TypeError，这里换成能看懂的说明；接口返回的错误显示服务端的消息。
export const handleError: HandleClientError = ({ error }) => {
	if (error instanceof ApiError) return { message: error.message };
	if (error instanceof TypeError) return { message: '连不上服务器。离线时只能打开读过的书。' };
};
