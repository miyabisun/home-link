<script lang="ts">
  import { airconText, outdoorText, placeText } from "../lib/admin";
  import { aborted, api, type Aircon } from "../lib/api";

  // The sensor and the air conditioner change on their own; the page follows while it is shown.
  const REFRESH_MS = 10_000;

  let aircon = $state<Aircon | undefined>();
  let pageState = $state<"loading" | "error" | "success">("loading");
  let error = $state("");
  let sending = $state(false);
  let address = $state("");
  let addressProblem = $state("");
  let locating = $state(false);
  // The settings start open until the home's place is registered.
  let settingsOpen = $state<boolean | undefined>();
  let controller: AbortController | undefined;

  const outdoor = $derived(aircon ? outdoorText(aircon.outdoor) : null);

  function show(next: Aircon) {
    aircon = next;
    settingsOpen ??= !next.location.registered;
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
    void api.location().then(
      (location) => (address = location.address ?? ""),
      () => {},
    );
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

  async function switchTo(on: boolean) {
    controller?.abort();
    sending = true;
    error = "";
    try {
      show(await api.updateAircon({ on }));
    } catch (e) {
      error = (e as Error).message;
    }
    sending = false;
  }

  async function saveAddress() {
    if (!address.trim()) return;
    locating = true;
    addressProblem = "";
    try {
      await api.updateLocation(address);
      await load();
    } catch (e) {
      addressProblem = (e as Error).message;
    }
    locating = false;
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
    <div class="switch-row">
      <span>自動運転</span>
      <button
        class="btn"
        type="button"
        aria-label="入り切り"
        aria-pressed={aircon.on}
        disabled={sending}
        onclick={() => void switchTo(!aircon!.on)}
      >
        {aircon.on ? "入" : "切"}
      </button>
    </div>

    {#if outdoor}<p class="outdoor">
        外気温 <strong>{outdoor.celsius}℃</strong>
        <time>{outdoor.time}</time>
      </p>{/if}

    {#each aircon.aircons as unit (unit.id)}
      <section class="aircon" aria-labelledby={`aircon-${unit.id}`}>
        <h2 id={`aircon-${unit.id}`}>
          <a href={`/devices/${unit.id}`}>{unit.name || "エアコン"}</a>
        </h2>
        <dl class="readings">
          <div class="reading">
            <dt>
              室温{#if unit.sensor}<small>{unit.sensor.name}</small>{/if}
            </dt>
            <dd>
              {#if unit.reading?.temperature != null}{unit.reading.temperature.toFixed(
                  1,
                )}<span>℃</span>{:else}—{/if}
            </dd>
          </div>
          <div class="reading">
            <dt>湿度</dt>
            <dd>
              {#if unit.reading?.humidity != null}{Math.round(
                  unit.reading.humidity,
                )}<span>%</span>{:else}—{/if}
            </dd>
          </div>
          <div class="reading">
            <dt>運転</dt>
            <dd class="text" class:danger={unit.state === "no_response"}>
              {airconText(unit)}
            </dd>
          </div>
        </dl>
        {#if unit.reason}<p class="reason" aria-live="polite">
            {unit.reason}
          </p>{/if}
      </section>
    {:else}
      <p class="state">エアコンがありません</p>
    {/each}

    <details class="settings" bind:open={settingsOpen}>
      <summary>設定</summary>
      <div class="field">
        <label for="aircon-address">住所</label>
        <input
          id="aircon-address"
          type="text"
          autocomplete="street-address"
          bind:value={address}
          disabled={locating}
          aria-invalid={addressProblem ? "true" : undefined}
          aria-describedby="aircon-place"
          onchange={() => void saveAddress()}
        />
        {#if addressProblem}<p class="field-error" role="alert">
            {addressProblem}
          </p>{/if}
        <p class="place" id="aircon-place">
          {#if locating}<span class="spinner" aria-hidden="true"></span>{/if}
          {placeText(aircon.location)}
          <span class="coords"
            >{aircon.location.latitude.toFixed(4)}, {aircon.location.longitude.toFixed(
              4,
            )}</span
          >
        </p>
      </div>
    </details>
  {/if}
</section>

<style lang="sass">
  .switch-row
    display: flex
    align-items: center
    justify-content: space-between
    gap: var(--sp-2)
    margin: 0 0 var(--sp-3)

    .btn
      flex-shrink: 0
      min-width: 52px
      height: 44px

  .outdoor
    margin: 0 0 var(--sp-3)
    color: var(--c-muted)
    font-size: var(--fs-sm)

    strong
      color: var(--c-on-surface)
      font-size: var(--fs-md)
      font-variant-numeric: tabular-nums

    time
      margin-left: var(--sp-1)
      font-variant-numeric: tabular-nums

  .aircon + .aircon
    margin-top: var(--sp-5)

  h2
    margin: 0 0 var(--sp-2)
    font-size: var(--fs-md)
    font-weight: 500
    overflow-wrap: anywhere

    a
      color: inherit

  .readings
    display: grid
    grid-template-columns: repeat(3, minmax(0, 1fr))
    gap: var(--sp-2)
    margin: 0

  .reason
    margin: var(--sp-3) 0 0
    font-size: var(--fs-sm)
    overflow-wrap: anywhere

  .settings
    margin-top: var(--sp-4)

    summary
      padding: var(--sp-3) 0
      cursor: pointer
      color: var(--c-muted)
      font-size: var(--fs-sm)

  .place
    margin: var(--sp-1) 0 0
    font-size: var(--fs-sm)
    overflow-wrap: anywhere

  .coords
    margin-left: var(--sp-1)
    color: var(--c-muted)
    font-variant-numeric: tabular-nums

  dt small
    margin-left: var(--sp-1)
    font-size: inherit

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
