<script lang="ts">
	// 设置项名字旁的问号按钮，点一下展开说明、再点收起。手机上没有悬停，所以不用 title 提示。
	// 说明由调用方放在这一行下面，用 controls 关联。
	import Icon from '#lib/Icon.svelte';

	let {
		open = $bindable(false),
		controls,
		topic
	}: { open?: boolean; controls: string; topic: string } = $props();
</script>

<button
	type="button"
	class="adm-help"
	aria-expanded={open}
	aria-controls={controls}
	aria-label="{topic}的说明"
	onclick={() => (open = !open)}
>
	<Icon name="help" size={17} />
</button>
