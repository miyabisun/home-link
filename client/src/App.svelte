<script lang="ts">
  import Header from "./lib/Header.svelte";
  import { initRouter, router } from "./lib/router.svelte";
  import Device from "./pages/Device.svelte";
  import Devices from "./pages/Devices.svelte";
  import Label from "./pages/Label.svelte";
  import Labels from "./pages/Labels.svelte";
  import Meter from "./pages/Meter.svelte";

  $effect(() => initRouter());
</script>

<svelte:head>
  <title>home-link</title>
</svelte:head>

<Header />

<main>
  {#if router.index === 1}
    {#key router.params.id}<Device id={router.params.id} />{/key}
  {:else if router.index === 2}
    <Labels />
  {:else if router.index === 3}
    {#key router.params.id}<Label id={router.params.id} />{/key}
  {:else if router.index === 4}
    <Meter />
  {:else}
    <Devices />
  {/if}
</main>
