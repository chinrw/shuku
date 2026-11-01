<script lang="ts">
	// 设置卡片的标题行：标题、可选的副标题和问号说明。
	import Help from './Help.svelte';

	let { title, sub, help }: { title: string; sub?: string; help?: string } = $props();

	let open = $state(false);
	const helpId = $props.id();
</script>

<div class="adm-card-head">
	<h2>{title}</h2>
	{#if help}<Help bind:open controls={helpId} topic={title} />{/if}
	{#if sub}<span class="adm-sub">{sub}</span>{/if}
	{#if help && open}<p class="adm-note adm-help-text" id={helpId}>{help}</p>{/if}
</div>
