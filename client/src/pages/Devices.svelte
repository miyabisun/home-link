<script lang="ts">
  import { deviceName, stateText, THREAD_DOWN, threadDown } from "../lib/admin";
  import { aborted, api, type Device } from "../lib/api";

  let devices = $state<Device[]>([]);
  let listState = $state<"loading" | "empty" | "error" | "success">("loading");
  let controller: AbortController | undefined;

  async function load() {
    controller?.abort();
    controller = new AbortController();
    try {
      devices = await api.devices(controller.signal);
      listState = devices.length === 0 ? "empty" : "success";
    } catch (error) {
      if (!aborted(error)) listState = "error";
    }
  }

  function onVisibilityChange() {
    if (document.visibilityState === "visible") void load();
  }

  $effect(() => {
    void load();
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      controller?.abort();
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  });
</script>

<section class="content" data-state={listState}>
  {#if listState === "loading"}
    <p class="state">
      <span class="spinner" aria-hidden="true"></span>読み込み中…
    </p>
  {:else if listState === "empty"}
    <p class="state">機器がありません</p>
  {:else if listState === "error"}
    <div class="state-wrap">
      <p class="state error">読み込みに失敗しました</p>
      <button class="btn" type="button" onclick={() => void load()}
        >再試行</button
      >
    </div>
  {:else}
    {#if threadDown(devices)}
      <p class="error-banner" role="alert">{THREAD_DOWN}</p>
    {/if}
    <ul class="cards">
      {#each devices as device (device.id)}
        {@const state = stateText(device.state)}
        <li>
          <a class="card" href={`/devices/${device.id}`}>
            <span class="name">{deviceName(device)}</span>
            {#if device.labels.length}
              <span class="note"
                >{device.labels.map((l) => l.name).join("・")}</span
              >
            {/if}
            {#if device.on}<span class="badge">入</span>{/if}
            {#if device.ignored}<span class="badge">無視</span>{/if}
            {#if state}
              <span class="badge" class:danger={device.state === "no_response"}
                >{state}</span
              >
            {/if}
          </a>
        </li>
      {/each}
    </ul>
  {/if}
</section>
