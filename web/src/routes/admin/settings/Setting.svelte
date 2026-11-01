<script lang="ts">
	// 设置页的一项：名字和问号在左，控件在右，手机上名字在上；说明展开在这一行下面。
	import type { Snippet } from 'svelte';
	import Help from './Help.svelte';

	let {
		label,
		id,
		help,
		children
	}: { label: string; id?: string; help?: string; children: Snippet } = $props();

	let open = $state(false);
	const helpId = $props.id();
</script>

<div class="adm-option">
	<div class="adm-option-name">
		{#if id}<label class="adm-label" for={id}>{label}</label>{:else}<span class="adm-label">{label}</span>{/if}
		{#if help}<Help bind:open controls={helpId} topic={label} />{/if}
	</div>
	<div class="adm-option-control">{@render children()}</div>
	{#if help && open}<p class="adm-note adm-help-text" id={helpId}>{help}</p>{/if}
</div>
