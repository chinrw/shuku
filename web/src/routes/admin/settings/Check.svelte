<script lang="ts">
	// 勾选框一行，可以带问号说明。
	import Help from './Help.svelte';

	let {
		checked = $bindable(false),
		label,
		help
	}: { checked?: boolean; label: string; help?: string } = $props();

	let open = $state(false);
	const helpId = $props.id();
</script>

<div class="adm-check-row">
	<label class="adm-check"><input type="checkbox" bind:checked />{label}</label>
	{#if help}<Help bind:open controls={helpId} topic={label} />{/if}
	{#if help && open}<p class="adm-note adm-help-text" id={helpId}>{help}</p>{/if}
</div>
