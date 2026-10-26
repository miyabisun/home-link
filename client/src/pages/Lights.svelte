<script lang="ts">
  import { outageText, type Outage, watchBadge, watchNote } from "../lib/admin";
  import { aborted, api, type Watch } from "../lib/api";

  // The server checks every light once a minute; the page follows while it is shown.
  const REFRESH_MS = 60_000;

  let watch = $state<Watch | undefined>();
  let outages = $state<Outage[]>([]);
  let pageState = $state<"loading" | "error" | "success">("loading");
  let now = $state(Math.floor(Date.now() / 1000));
  let sending = $state(false);
  let sent = $state<{ ok: boolean; text: string } | undefined>();
  let controller: AbortController | undefined;

  async function load() {
    controller?.abort();
    controller = new AbortController();
    try {
      const [current, history] = await Promise.all([
        api.watch(controller.signal),
        api.outages(controller.signal),
      ]);
      watch = current;
      outages = history.outages.toReversed();
      now = Math.floor(Date.now() / 1000);
      pageState = "success";
    } catch (error) {
      if (!aborted(error)) pageState = "error";
    }
  }

  async function test() {
    sending = true;
    sent = undefined;
    try {
      await api.testNotice();
      sent = { ok: true, text: "送りました" };
    } catch (error) {
      sent = { ok: false, text: (error as Error).message };
    } finally {
      sending = false;
    }
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

<div class="sub-header"><h1>照明の接続</h1></div>

<section class="content" data-state={pageState}>
  {#if pageState === "loading"}
    <p class="state">
      <span class="spinner" aria-hidden="true"></span>読み込み中…
    </p>
  {:else if pageState === "error" || !watch}
    <div class="state-wrap">
      <p class="state error">読み込みに失敗しました</p>
      <button class="btn" type="button" onclick={() => void load()}
        >再試行</button
      >
    </div>
  {:else}
    {#if watch.server_outage}
      <p class="error-banner" role="alert">
        matterjs-serverに届きません {outageText(watch.server_outage, now)}
      </p>
    {/if}
    {#if watch.checked_at === null}
      <p class="state">まだ確かめていません</p>
    {:else if watch.lights.length === 0}
      <p class="state">照明がありません</p>
    {:else}
      <ul class="cards">
        {#each watch.lights as light (light.node_id === null ? `device/${light.device_id}` : `${light.node_id}/${light.endpoint}`)}
          {@const badge = watchBadge(light)}
          <li class="card">
            <span class="name">{light.name}</span>
            <span class="note">{watchNote(light, now)}</span>
            {#if badge}
              <span class="badge danger">{badge}</span>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}

    <details class="history">
      <summary>14日の履歴（{outages.length}件）</summary>
      {#if outages.length === 0}
        <p class="note">途切れた記録はありません</p>
      {:else}
        <ul>
          {#each outages as outage (`${outage.node_id}/${outage.endpoint}/${outage.name}/${outage.started_at}`)}
            <li>
              <span class="name">{outage.name}</span>
              <span class="note">{outageText(outage, now)}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </details>

    <div class="notify">
      {#if sent}
        <p class="result" class:error={!sent.ok} role="status">{sent.text}</p>
      {:else if !watch.notify}
        <p class="result" role="status">知らせの送り先が未設定です</p>
      {/if}
      <button
        class="btn"
        type="button"
        disabled={sending || !watch.notify}
        onclick={() => void test()}>試しに知らせる</button
      >
    </div>
  {/if}
</section>

<style lang="sass">
  li.card:hover
    background: var(--c-surface-raised)

  .history
    margin-top: var(--sp-3)

    summary
      padding: 12px 0
      cursor: pointer
      font-size: var(--fs-sm)

    ul
      display: flex
      flex-direction: column
      gap: var(--sp-1)
      margin: 0
      padding: 0
      list-style: none

    li
      display: flex
      flex-wrap: wrap
      gap: 0 var(--sp-2)
      font-size: var(--fs-sm)

    .name
      overflow-wrap: anywhere

  .note
    color: var(--c-muted)
    font-size: var(--fs-xs)

  .notify
    display: flex
    flex-wrap: wrap
    align-items: center
    justify-content: flex-end
    gap: var(--sp-2)
    margin-top: var(--sp-3)

  .result
    margin: 0
    color: var(--c-muted)
    font-size: var(--fs-sm)

    &.error
      color: var(--c-danger)
</style>
