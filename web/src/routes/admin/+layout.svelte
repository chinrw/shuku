<script lang="ts">
	import { page } from '$app/state';
	import { adminStatus } from '#lib/adminStatus.svelte.ts';
	import Icon from '#lib/Icon.svelte';
	import { clock, duration } from '#lib/format.ts';
	import './admin.css';

	let { children } = $props();

	$effect(() => adminStatus.poll());

	let status = $derived(adminStatus.current);
	let problems = $derived(
		status
			? status.scan.issues.failed + status.scan.issues.unreadable + status.scan.issues.missing
			: 0
	);
	let busy = $derived(
		status?.scan.running ? '扫描中' : status?.hash.running ? '补算中' : ''
	);

	const sections = [
		{ href: '/admin', label: '概览', icon: 'overview' },
		{ href: '/admin/tasks', label: '任务', icon: 'refresh' },
		{ href: '/admin/issues', label: '问题', icon: 'alert' },
		{ href: '/admin/tags', label: '标签', icon: 'tag' },
		{ href: '/admin/logs', label: '日志', icon: 'log' },
		{ href: '/admin/settings', label: '设置', icon: 'settings' }
	] as const;
</script>

<svelte:head><title>控制台 · 书库</title></svelte:head>

<div class="console">
	<nav aria-label="控制台分区">
		<a class="back" href="/"><Icon name="back" size={16} />书库</a>
		<div class="heading">控制台</div>
		<div class="links">
			{#each sections as section (section.href)}
				<a
					href={section.href}
					aria-current={page.url.pathname === section.href ? 'page' : undefined}
				>
					<Icon name={section.icon} size={18} />
					{section.label}
					{#if section.href === '/admin/tasks' && busy}
						<span class="badge busy">{busy}</span>
					{:else if section.href === '/admin/issues' && problems > 0}
						<span class="badge warn">{problems}</span>
					{/if}
				</a>
			{/each}
		</div>
		{#if status}
			<p class="about">
				Shuku {status.server.version}<br />
				已运行 {duration(Date.now() - status.server.started_at)}<br />
				局域网直连，无认证
			</p>
		{/if}
	</nav>
	<main>
		{#if adminStatus.error}
			<p class="stale error">
				取不到状态：{adminStatus.error}{#if adminStatus.updated}，下面是 {clock(adminStatus.updated)} 的数据{/if}
			</p>
		{/if}
		{@render children()}
	</main>
</div>

<style>
	.console {
		display: flex;
		flex-wrap: wrap;
		min-height: 100vh;
	}

	nav {
		flex: 1 1 14.5rem;
		max-width: 14.5rem;
		box-sizing: border-box;
		padding: 1.25rem 0.9rem;
		padding-top: max(1.25rem, env(safe-area-inset-top));
		background: var(--sunken);
		display: flex;
		flex-direction: column;
		gap: 1.1rem;
	}

	.back {
		display: inline-flex;
		align-items: center;
		gap: 0.3rem;
		padding: 0 0.4rem;
		color: var(--muted);
		font-size: 0.85rem;
		text-decoration: none;
	}

	.heading {
		padding: 0 0.5rem;
		font-size: 1.25rem;
		font-weight: 700;
	}

	.links {
		display: flex;
		flex-direction: column;
		gap: 2px;
	}

	.links a {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		min-height: 2.5rem;
		padding: 0 0.6rem;
		border-radius: 6px;
		text-decoration: none;
	}

	.links a:hover {
		color: var(--link);
	}

	.links a[aria-current='page'] {
		background: var(--surface);
		color: var(--fg);
		font-weight: 600;
	}

	.badge {
		margin-left: auto;
		padding: 1px 7px;
		border-radius: 999px;
		color: #1c1c1c;
		font-size: 0.7rem;
		font-weight: 600;
	}

	.busy {
		background: var(--accent);
		color: #fff;
	}

	.warn {
		background: var(--new);
	}

	.about {
		margin: auto 0 0;
		padding: 0.6rem;
		border-radius: 8px;
		background: var(--bg);
		color: var(--muted);
		font-size: 0.75rem;
		line-height: 1.6;
	}

	main {
		flex: 999 1 35rem;
		min-width: 0;
		box-sizing: border-box;
		padding: 1.4rem 1.75rem 2.5rem;
	}

	.stale {
		margin: 0 0 1rem;
	}

	/* 窄屏时导航变成顶部一排可横滑的分区标签。 */
	@media (max-width: 800px) {
		nav {
			max-width: none;
			flex-basis: 100%;
			padding: 0.75rem 1rem 0.6rem;
			padding-top: max(0.75rem, env(safe-area-inset-top));
			gap: 0.5rem;
			background: var(--bg);
			border-bottom: 1px solid var(--border);
		}

		.links {
			flex-direction: row;
			overflow-x: auto;
			gap: 0.5rem;
			padding-bottom: 0.2rem;
		}

		.links a {
			flex: none;
			min-height: 2.25rem;
			padding: 0 0.85rem;
			border: 1px solid var(--chip-border);
			border-radius: 999px;
			gap: 0.4rem;
		}

		.links a :global(svg) {
			display: none;
		}

		.links a[aria-current='page'] {
			background: var(--fg);
			color: var(--bg);
			border-color: var(--fg);
		}

		.badge {
			margin-left: 0.2rem;
		}

		.about {
			display: none;
		}

		main {
			padding: 1rem 1rem 2rem;
		}
	}
</style>
