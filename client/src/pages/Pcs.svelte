<script lang="ts">
  import { aborted, api, type Pc } from "../lib/api";

  // A PC reports its own changes; the page follows while it is shown.
  const REFRESH_MS = 15_000;

  let pcs = $state<Pc[]>([]);
  let listState = $state<"loading" | "empty" | "error" | "success">("loading");
  let sending = $state<string | undefined>();
  let error = $state("");
  let controller: AbortController | undefined;

  async function load() {
    controller?.abort();
    controller = new AbortController();
    try {
      pcs = await api.pcs(controller.signal);
      listState = pcs.length === 0 ? "empty" : "success";
    } catch (e) {
      if (!aborted(e)) listState = "error";
    }
  }

  async function send(pc: Pc, sleep: boolean) {
    sending = pc.name;
    error = "";
    try {
      const result = await api.sleepPc(pc.name, sleep);
      pc.sleeping = result.sleeping;
    } catch (e) {
      error = (e as Error).message;
      void load();
    }
    sending = undefined;
  }

  function onVisibilityChange() {
    if (document.visibilityState === "visible") void load();
  }

  $effect(() => {
    void load();
    const timer = setInterval(() => {
      if (document.visibilityState === "visible") void load();
    }, REFRESH_MS);
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      clearInterval(timer);
      controller?.abort();
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  });
</script>

<div class="sub-header"><h1>PC</h1></div>

<section class="content" data-state={listState}>
  {#if error}<p class="error-banner" role="alert">{error}</p>{/if}
  {#if listState === "loading"}
    <p class="state">
      <span class="spinner" aria-hidden="true"></span>読み込み中…
    </p>
  {:else if listState === "empty"}
    <p class="state">つながっているPCはありません</p>
  {:else if listState === "error"}
    <div class="state-wrap">
      <p class="state error">読み込みに失敗しました</p>
      <button class="btn" type="button" onclick={() => void load()}
        >再試行</button
      >
    </div>
  {:else}
    <ul class="cards">
      {#each pcs as pc (pc.name)}
        <li class="card">
          <span class="name">{pc.name}</span>
          {#if pc.sleeping === null}
            <span class="actions">
              <button
                class="btn"
                type="button"
                disabled={sending === pc.name}
                onclick={() => void send(pc, true)}>スリープ</button
              >
              <button
                class="btn"
                type="button"
                disabled={sending === pc.name}
                onclick={() => void send(pc, false)}>復帰</button
              >
            </span>
          {:else}
            <button
              class="btn"
              type="button"
              aria-label="スリープ"
              aria-pressed={pc.sleeping}
              disabled={sending === pc.name}
              onclick={() => void send(pc, !pc.sleeping)}
            >
              {pc.sleeping ? "スリープ中" : "動作中"}
            </button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style lang="sass">
  .card
    flex-wrap: nowrap
    align-items: center

  li.card:hover
    background: var(--c-surface-raised)

  .name
    flex: 1
    min-width: 0
    overflow-wrap: anywhere

  .actions
    display: flex
    gap: var(--sp-2)

  .btn
    flex-shrink: 0
</style>
