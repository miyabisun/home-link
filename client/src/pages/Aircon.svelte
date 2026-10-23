<script lang="ts">
  import {
    airconText,
    TARGET_MAX,
    TARGET_MIN,
    targetError,
  } from "../lib/admin";
  import { aborted, api, type Aircon } from "../lib/api";

  // The meter and the air conditioner change on their own; the page follows while it is shown.
  const REFRESH_MS = 10_000;

  let aircon = $state<Aircon | undefined>();
  let pageState = $state<"loading" | "error" | "success">("loading");
  // A number input binds a number, or null while it is empty.
  let target = $state<number | null>(null);
  let targetProblem = $state("");
  let error = $state("");
  let sending = $state(false);
  let controller: AbortController | undefined;

  function show(next: Aircon) {
    aircon = next;
    // A target being typed is not overwritten by a reload.
    if (document.activeElement?.id !== "aircon-target") target = next.target;
  }

  async function load() {
    controller?.abort();
    controller = new AbortController();
    try {
      show(await api.aircon(controller.signal));
      pageState = "success";
    } catch (e) {
      if (!aborted(e)) pageState = "error";
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

  async function update(patch: Parameters<typeof api.updateAircon>[0]) {
    controller?.abort();
    sending = true;
    error = "";
    try {
      show(await api.updateAircon(patch));
    } catch (e) {
      error = (e as Error).message;
    }
    sending = false;
  }

  function saveTarget() {
    targetProblem = targetError(String(target ?? "")) ?? "";
    if (!targetProblem && aircon && target !== aircon.target)
      void update({ target: Number(target) });
  }
</script>

<div class="sub-header"><h1>エアコン</h1></div>

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
  {:else if aircon}
    {#if error}<p class="error-banner" role="alert">{error}</p>{/if}
    <div class="field target-row">
      <div class="grow">
        <label for="aircon-target">目標</label>
        <div class="unit">
          <input
            id="aircon-target"
            type="number"
            inputmode="numeric"
            min={TARGET_MIN}
            max={TARGET_MAX}
            step="1"
            bind:value={target}
            aria-invalid={targetProblem ? "true" : undefined}
            aria-describedby={targetProblem ? "aircon-target-error" : undefined}
            onchange={saveTarget}
          />
          <span>℃</span>
        </div>
      </div>
      <button
        class="btn"
        type="button"
        aria-label="入り切り"
        aria-pressed={aircon.on}
        disabled={sending}
        onclick={() => void update({ on: !aircon!.on })}
      >
        {aircon.on ? "入" : "切"}
      </button>
    </div>
    {#if targetProblem}<p
        class="field-error target-error"
        id="aircon-target-error"
        role="alert"
      >
        {targetProblem}
      </p>{/if}

    <dl class="readings">
      <div class="reading">
        <dt>室温</dt>
        <dd>
          {#if aircon.reading}{aircon.reading.temperature.toFixed(1)}<span
              >℃</span
            >{:else}—{/if}
        </dd>
      </div>
      <div class="reading">
        <dt>湿度</dt>
        <dd>
          {#if aircon.reading}{aircon.reading.humidity}<span>%</span
            >{:else}—{/if}
        </dd>
      </div>
      <div class="reading wide">
        <dt>{aircon.device?.name || "エアコン"}</dt>
        <dd class="text" class:danger={aircon.device?.state === "no_response"}>
          {airconText(aircon.device)}
        </dd>
      </div>
    </dl>
  {/if}
</section>

<style lang="sass">
  .target-row
    display: flex
    align-items: flex-end
    gap: var(--sp-2)

    .grow
      flex: 1
      min-width: 0

    .btn
      flex-shrink: 0
      min-width: 52px
      height: 44px

  .unit
    display: flex
    align-items: center
    gap: var(--sp-1)

    input
      width: 6em

    span
      color: var(--c-muted)

  .target-error
    margin: calc(var(--sp-3) * -1) 0 var(--sp-4)

  .readings
    display: grid
    grid-template-columns: repeat(2, minmax(0, 1fr))
    gap: var(--sp-2)
    margin: 0

  .wide
    grid-column: 1 / -1

  @media (min-width: 768px)
    .readings
      grid-template-columns: repeat(3, minmax(0, 1fr))

    .wide
      grid-column: auto

  .reading
    padding: 10px
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    background: var(--c-surface-raised)

  dt
    color: var(--c-muted)
    font-size: var(--fs-xs)
    line-height: 1.4
    overflow-wrap: anywhere

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

    &.danger
      color: var(--c-danger)
</style>
