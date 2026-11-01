<script lang="ts">
	// 打开了“网页端也需要令牌”以后，没登录的设备会转到这里。令牌在控制台的设置页或者
	// 用命令行 `shuku token create <设备名>` 新建。桌面和 Android 客户端打开服务器时也经过
	// 这里，令牌在地址的片段里，自动登录。
	import { goto, replaceState } from '$app/navigation';
	import { page } from '$app/state';
	import { ApiError } from '#lib/api.ts';
	import { login } from '#lib/auth.ts';
	import { handoffToken, serverListHref } from '#lib/nativeApp.ts';
	import { onMount } from 'svelte';

	let token = $state('');
	let busy = $state(false);
	let error = $state('');
	// 客户端里令牌失效时停在这里，要能回到服务器列表换令牌或者换服务器。
	const serverList = serverListHref(`${page.url.origin}/`);

	/** 只回到本站的路径，`next` 是地址栏里的参数，不能让它把人带到别的网站。 */
	function target() {
		const next = page.url.searchParams.get('next') ?? '/';
		return next.startsWith('/') && !next.startsWith('//') ? next : '/';
	}

	onMount(() => {
		// 只在客户端里接受。浏览器里别人发来的 /login#token=… 链接不能悄悄把这台设备登录成
		// 别的设备。
		const handed = window.shukuApp ? handoffToken(page.url.hash) : null;
		if (handed === null) return;
		// 令牌不留在地址栏和历史记录里。
		replaceState(page.url.pathname + page.url.search, page.state);
		token = handed;
		void submit();
	});

	async function submit(event?: SubmitEvent) {
		event?.preventDefault();
		if (!token.trim() || busy) return;
		busy = true;
		error = '';
		try {
			await login(token.trim());
			await goto(target(), { replaceState: true, refreshAll: true });
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
		{#if serverList}<a class="back" href={serverList}>返回服务器列表</a>{/if}
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
		color: var(--on-accent);
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

	.back {
		justify-self: center;
		color: var(--link);
		font-size: 0.9rem;
	}
</style>
