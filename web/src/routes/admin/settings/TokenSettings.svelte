<script lang="ts">
	// 控制台设置页的“设备令牌”卡片：新建和吊销令牌，这台设备登录或退出，
	// 以及“网页端也需要令牌”的开关。新建的令牌只显示一次。
	import { onMount } from 'svelte';
	import { ApiError } from '#lib/api.ts';
	import {
		authStatus,
		createDeviceToken,
		deviceTokens,
		login,
		logout,
		revokeDeviceToken,
		setRequireToken,
		type AuthStatus,
		type DeviceToken
	} from '#lib/auth.ts';
	import { ago } from '#lib/time.ts';

	let status = $state<AuthStatus | null>(null);
	let tokens = $state<DeviceToken[]>([]);
	let name = $state('');
	/** 刚新建的令牌，只在这次显示。 */
	let created = $state<{ name: string; token: string } | null>(null);
	let busy = $state(false);
	let error = $state('');

	function describe(cause: unknown) {
		return cause instanceof ApiError ? cause.message : '网络错误';
	}

	async function reload() {
		try {
			[status, tokens] = await Promise.all([authStatus(), deviceTokens()]);
		} catch (cause) {
			error = describe(cause);
		}
	}

	onMount(reload);

	async function run(action: () => Promise<unknown>) {
		if (busy) return;
		busy = true;
		error = '';
		try {
			await action();
			await reload();
		} catch (cause) {
			error = describe(cause);
		} finally {
			busy = false;
		}
	}

	function create(event: SubmitEvent) {
		event.preventDefault();
		const trimmed = name.trim();
		if (!trimmed) return;
		run(async () => {
			const token = await createDeviceToken(trimmed);
			created = { name: token.name, token: token.token };
			copied = '';
			name = '';
		});
	}

	let copied = $state('');
	let tokenNode = $state<HTMLElement>();

	/** 局域网里用 HTTP 时 navigator.clipboard 不可用，退回到选中文本再 execCommand。 */
	async function copy(text: string) {
		try {
			if (navigator.clipboard) {
				await navigator.clipboard.writeText(text);
				copied = '已复制';
				return;
			}
		} catch {
			// 落到下面的办法。
		}
		const selection = getSelection();
		if (tokenNode && selection) {
			selection.selectAllChildren(tokenNode);
			copied = document.execCommand('copy') ? '已复制' : '已选中，请手动复制';
		} else {
			copied = '请手动复制';
		}
	}

	function revoke(token: DeviceToken) {
		const self = status?.device?.id === token.id;
		const warning = self ? '这是这台设备正在用的令牌，吊销后这台设备要重新登录。' : '';
		if (!confirm(`吊销“${token.name}”的令牌？${warning}`)) return;
		run(() => revokeDeviceToken(token.id));
	}
</script>

<section class="adm-card">
	<h2>设备令牌</h2>
	<p class="adm-note">
		每台设备一个令牌，可以单独吊销。agent 通过 /mcp 访问时总要带令牌（Authorization: Bearer）；
		网页端和 API 默认不要，打开下面的开关以后也要。
	</p>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#if status}
		<div class="option">
			<span class="adm-label">网页端也需要令牌</span>
			<div class="adm-tabs" role="group" aria-label="网页端也需要令牌">
				<button
					class="adm-tab"
					aria-pressed={status.require_token}
					disabled={busy || !status.device}
					onclick={() => run(() => setRequireToken(true))}>开</button
				>
				<button
					class="adm-tab"
					aria-pressed={!status.require_token}
					disabled={busy}
					onclick={() => run(() => setRequireToken(false))}>关</button
				>
			</div>
		</div>
		<div class="option">
			<span class="adm-label">这台设备</span>
			<div class="device">
				{#if status.device}
					已用“{status.device.name}”的令牌登录
					<button class="adm-button" disabled={busy} onclick={() => run(logout)}>退出</button>
				{:else}
					没有登录{#if !status.require_token}，要打开开关先在这台设备上登录{/if}
				{/if}
			</div>
		</div>
	{/if}

	{#if created}
		<div class="created">
			<p>“{created.name}”的令牌，只显示这一次：</p>
			<code class="adm-mono" bind:this={tokenNode}>{created.token}</code>
			<div class="adm-actions">
				<button class="adm-button" onclick={() => created && copy(created.token)}>复制</button>
				{#if copied}<span class="adm-note">{copied}</span>{/if}
				<button
					class="adm-button primary"
					disabled={busy}
					onclick={() => created && run(() => login(created!.token))}>在这台设备上登录</button
				>
				<button class="adm-button" onclick={() => (created = null)}>收起</button>
			</div>
		</div>
	{/if}

	{#if tokens.length}
		<div class="adm-scroll">
			<table class="adm-table">
				<thead><tr><th>设备</th><th>新建</th><th>最近使用</th><th></th></tr></thead>
				<tbody>
					{#each tokens as token (token.id)}
						<tr>
							<td>{token.name}{#if status?.device?.id === token.id}（这台设备）{/if}</td>
							<td>{ago(token.created_at)}</td>
							<td>{token.last_used_at ? ago(token.last_used_at) : '从未使用'}</td>
							<td class="num">
								<button class="adm-button danger" disabled={busy} onclick={() => revoke(token)}>吊销</button>
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}

	<form class="new" onsubmit={create}>
		<input bind:value={name} placeholder="设备名，例如 客厅 iPad、Claude" aria-label="新令牌的设备名" maxlength="100" />
		<button class="adm-button primary" disabled={busy || !name.trim()}>新建令牌</button>
	</form>
</section>

<style>
	.option {
		display: grid;
		grid-template-columns: minmax(6rem, 10rem) minmax(0, 1fr);
		align-items: center;
		gap: 0.75rem;
		padding: 0.6rem 0;
		border-top: 1px solid var(--border);
	}

	.device {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.75rem;
	}

	.created {
		display: grid;
		gap: 0.5rem;
		padding: 0.8rem;
		border: 1px solid var(--accent);
		border-radius: 8px;
	}

	.created p {
		margin: 0;
	}

	.new {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
		margin-top: 0.6rem;
	}

	.new input {
		flex: 1 1 14rem;
		min-width: 0;
		height: 2.5rem;
		padding: 0 0.75rem;
		border: 1px solid var(--border);
		border-radius: 8px;
		background: var(--bg);
		color: inherit;
		font: inherit;
	}

	.error {
		color: var(--danger);
	}
</style>
