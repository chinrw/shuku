declare global {
	namespace App {
		// 浅层路由（$app/navigation 的 pushState）记在历史记录里的状态：打开面板、抽屉时
		// 加一条记录，手机的返回手势和浏览器的后退只关掉它们，不离开当前页面。
		interface PageState {
			panel?: 'detail' | 'settings';
			drawer?: number;
		}
	}
}

export {};
