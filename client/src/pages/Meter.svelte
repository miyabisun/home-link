<script lang="ts">
  import { meterState, receivedTime } from "../lib/admin";
  import { aborted, api, type Meter } from "../lib/api";

  // The meter advertises every few seconds; the page follows while it is shown.
  const REFRESH_MS = 10_000;

  let meter = $state<Meter | undefined>();
  let pageState = $state<"loading" | "error" | "success">("loading");
  let controller: AbortController | undefined;

  async function load() {
    controller?.abort();
    controller = new AbortController();
    try {
      meter = await api.meter(controller.signal);
      pageState = "success";
    } catch (error) {
      if (!aborted(error)) pageState = "error";
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

  const status = $derived(meter ? meterState(meter) : undefined);
</script>

<div class="sub-header"><h1>温度計</h1></div>

<section class="content" data-state={pageState}>
  {#if pageState === "loading"}
    <p class="state">
      <span class="spinner" aria-hidden="true"></span>読み込み中…
    </p>
  {:else if pageState === "error"}
    <div class="state-wrap">
      <p class="state error">読み込みに失敗しました</p>
      <button class="btn" type="button" onclick={() => void load()}
        >再試行</button
      >
    </div>
  {:else if status === "unset"}
    <p class="state">温度計が設定されていません</p>
  {:else if status === "waiting"}
    <p class="state">まだ受信していません</p>
  {:else if status === "stale" && meter?.received_at}
    <div class="state-wrap">
      <p class="state error">受信が途絶えています</p>
      <p class="note">最後の受信 {receivedTime(meter.received_at, true)}</p>
    </div>
  {:else if meter?.reading && meter.received_at}
    <dl class="readings">
      <div class="reading">
        <dt>温度</dt>
        <dd>{meter.reading.temperature.toFixed(1)}<span>℃</span></dd>
      </div>
      <div class="reading">
        <dt>湿度</dt>
        <dd>{meter.reading.humidity}<span>%</span></dd>
      </div>
      <div class="reading">
        <dt>CO2</dt>
        <dd>{meter.reading.co2}<span>ppm</span></dd>
      </div>
      <div class="reading">
        <dt>電池</dt>
        <dd>{meter.reading.battery}<span>%</span></dd>
      </div>
    </dl>
    <p class="note">{receivedTime(meter.received_at, false)} 受信</p>
  {/if}
</section>

<style lang="sass">
  .readings
    display: grid
    grid-template-columns: repeat(2, minmax(0, 1fr))
    gap: var(--sp-2)
    margin: 0

  @media (min-width: 768px)
    .readings
      grid-template-columns: repeat(4, minmax(0, 1fr))

  .reading
    padding: 10px
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    background: var(--c-surface-raised)

  dt
    color: var(--c-muted)
    font-size: var(--fs-xs)
    line-height: 1.4

  dd
    margin: 0
    font-size: var(--fs-xl)
    font-weight: 600
    line-height: 1.3
    font-variant-numeric: tabular-nums

    span
      margin-left: 2px
      color: var(--c-muted)
      font-size: var(--fs-sm)
      font-weight: 400

  .note
    margin: var(--sp-2) 0 0
    color: var(--c-muted)
    font-size: var(--fs-xs)
    text-align: right

  .state-wrap .note
    margin: 0
    text-align: center
</style>
