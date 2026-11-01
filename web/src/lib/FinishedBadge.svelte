<script lang="ts">
	import Icon from './Icon.svelte';

	// 封面左上角“读完”的角标，书库的卡片和最近阅读用。看的是读到过最后一页
	// （进度的 finished），重读到前面也还在。父元素要是定位元素。
</script>

<span class="finished" title="读到过最后一页"><Icon name="check" size={12} />读完</span>

<style>
	.finished {
		position: absolute;
		top: 0.3rem;
		left: 0.3rem;
		display: flex;
		align-items: center;
		gap: 0.15rem;
		padding: 0.05rem 0.35rem 0.05rem 0.25rem;
		border-radius: 4px;
		background: rgb(0 0 0 / 0.6);
		color: #eee;
		font-size: 0.72rem;
		line-height: 1.4;
	}
</style>
