// 设备令牌和登录，接口见 docs/api.md 的“认证”一节。
import { ApiError } from './api';

export interface Device {
	id: number;
	name: string;
}

export interface AuthStatus {
	/** 网页端和 API 也需要令牌。 */
	require_token: boolean;
	/** 这台设备登录用的令牌；没登录时为 `null`。 */
	device: Device | null;
}

export interface DeviceToken extends Device {
	/** Unix 毫秒。 */
	created_at: number;
	last_used_at: number | null;
}

/** 接口返回 401 时转到登录页，登录后回到原来的地址。已经在登录页时不转。 */
export function redirectToLogin() {
	if (location.pathname === '/login') return;
	const next = location.pathname + location.search;
	location.assign(`/login?${new URLSearchParams({ next })}`);
}

async function call<T>(path: string, init?: RequestInit): Promise<T> {
	const response = await fetch(path, init);
	if (!response.ok) {
		const body = await response.json().catch(() => null);
		throw new ApiError(response.status, body?.error ?? response.statusText);
	}
	return response.status === 204 ? (undefined as T) : response.json();
}

const json = (method: string, body: unknown): RequestInit => ({
	method,
	headers: { 'content-type': 'application/json' },
	body: JSON.stringify(body)
});

export const authStatus = () => call<AuthStatus>('/api/auth');
export const setRequireToken = (require_token: boolean) =>
	call<{ require_token: boolean }>('/api/auth', json('PUT', { require_token }));
export const login = (token: string) =>
	call<{ device: Device }>('/api/auth/session', json('POST', { token }));
export const logout = () => call<void>('/api/auth/session', { method: 'DELETE' });
export const deviceTokens = async () =>
	(await call<{ tokens: DeviceToken[] }>('/api/admin/tokens')).tokens;
/** 返回的 `token` 只有这一次能看到。 */
export const createDeviceToken = (name: string) =>
	call<DeviceToken & { token: string }>('/api/admin/tokens', json('POST', { name }));
export const revokeDeviceToken = (id: number) =>
	call<void>(`/api/admin/tokens/${id}`, { method: 'DELETE' });
