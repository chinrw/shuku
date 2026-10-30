<script lang="ts">
	import { page } from '$app/state';
</script>

<main>
	<h1>{page.status}</h1>
	<p class="error">{page.error?.message}</p>
	<a href="/">返回书库</a>
</main>

<style>
	main {
		padding: 2rem;
	}
</style>
