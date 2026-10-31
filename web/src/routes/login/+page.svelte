<script lang="ts">
	// 打开了“网页端也需要令牌”以后，没登录的设备会转到这里。令牌在控制台的设置页或者
	// 用命令行 `shuku token create <设备名>` 新建。
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { ApiError } from '#lib/api.ts';
	import { login } from '#lib/auth.ts';

	let token = $state('');
	let busy = $state(false);
	let error = $state('');

	/** 只回到本站的路径，`next` 是地址栏里的参数，不能让它把人带到别的网站。 */
	function target() {
		const next = page.url.searchParams.get('next') ?? '/';
		return next.startsWith('/') && !next.startsWith('//') ? next : '/';
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (!token.trim() || busy) return;
		busy = true;
		error = '';
		try {
			await login(token.trim());
			await goto(target(), { replaceState: true, invalidateAll: true });
		} catch (cause) {
			error = cause instanceof ApiError ? cause.message : '网络错误';
		} finally {
			busy = false;
		}
	}
</script>

<svelte:head>
	<title>登录 · Shuku</title>
</svelte:head>

<main>
	<form onsubmit={submit}>
		<h1>Shuku</h1>
		<p>这个书库需要设备令牌。在已经登录的设备上打开控制台的设置页新建一个，或者在服务器上运行
			<code>shuku token create 设备名</code>。</p>
		<input
			bind:value={token}
			placeholder="shuku_…"
			aria-label="设备令牌"
			autocomplete="off"
			spellcheck="false"
		/>
		<button disabled={!token.trim() || busy}>{busy ? '正在登录…' : '登录'}</button>
		{#if error}<p class="error" role="alert">{error}</p>{/if}
	</form>
</main>

<style>
	main {
		display: grid;
		place-items: center;
		min-height: 100dvh;
		padding: 1rem;
	}

	form {
		display: grid;
		gap: 0.9rem;
		width: min(26rem, 100%);
		padding: 1.5rem;
		border-radius: 12px;
		background: var(--surface);
	}

	h1 {
		margin: 0;
		font-size: 1.4rem;
	}

	p {
		margin: 0;
		color: var(--muted);
		font-size: 0.9rem;
		line-height: 1.6;
	}

	input {
		height: 2.6rem;
		padding: 0 0.75rem;
		border: 1px solid var(--border);
		border-radius: 8px;
		background: var(--bg);
		color: inherit;
		font: 0.85rem ui-monospace, 'SF Mono', Menlo, Consolas, monospace;
	}

	button {
		height: 2.6rem;
		border: 1px solid var(--accent);
		border-radius: 8px;
		background: var(--accent);
		color: white;
		font: inherit;
		font-weight: 600;
		cursor: pointer;
	}

	button:disabled {
		opacity: 0.55;
		cursor: default;
	}

	.error {
		color: var(--danger);
	}
</style>
