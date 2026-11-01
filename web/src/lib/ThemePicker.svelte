<script lang="ts">
	import { settings } from '#lib/settings.svelte.ts';
	import { themeGroups, type Scheme } from '#lib/themes.ts';

	// 阅读器的阅读设置和控制台的“这台设备”共用。选跟随系统时再分别选深色和浅色用哪一套，
	// 这样任何一个系列都能跟着系统切换。
	const prefs = settings();

	function choose(key: 'theme' | 'darkTheme' | 'lightTheme', value: string) {
		prefs[key] = value;
		prefs.save();
	}

	const schemes: [Scheme, 'darkTheme' | 'lightTheme', string][] = [
		['dark', 'darkTheme', '深色时用'],
		['light', 'lightTheme', '浅色时用']
	];
</script>

{#snippet options(scheme?: Scheme)}
	{#each themeGroups(scheme) as group (group.family)}
		{#if group.family}
			<optgroup label={group.family}>
				{#each group.themes as theme (theme.id)}<option value={theme.id}>{theme.label}</option>{/each}
			</optgroup>
		{:else}
			{#each group.themes as theme (theme.id)}<option value={theme.id}>{theme.label}</option>{/each}
		{/if}
	{/each}
{/snippet}

<div class="picker">
	<select
		value={prefs.theme}
		onchange={(event) => choose('theme', event.currentTarget.value)}
		aria-label="配色"
	>
		<option value="system">跟随系统</option>
		{@render options()}
	</select>
	{#if prefs.theme === 'system'}
		{#each schemes as [scheme, key, label] (scheme)}
			<label>
				<span>{label}</span>
				<select value={prefs[key]} onchange={(event) => choose(key, event.currentTarget.value)}>
					{@render options(scheme)}
				</select>
			</label>
		{/each}
	{/if}
</div>

<style>
	.picker {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 0.4rem;
	}

	label {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		color: var(--muted);
		font-size: 0.85rem;
	}

	select {
		padding: 0.3rem 0.5rem;
		border: 1px solid var(--border);
		border-radius: 6px;
		background: var(--surface);
		color: var(--fg);
		font-size: 0.85rem;
	}
</style>
