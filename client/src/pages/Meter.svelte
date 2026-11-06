<script lang="ts">
  import Attention from "../lib/Attention.svelte";
  import Icon from "../lib/Icon.svelte";
  import {
    adapterText,
    type Attention as Item,
    attentionHref,
    deviceName,
    deviceNameError,
    meterState,
    receivedTime,
  } from "../lib/admin";
  import { aborted, api, type Device, type Meter } from "../lib/api";

  // The meter advertises every few seconds; the page follows while it is shown.
  const REFRESH_MS = 10_000;

  let meter = $state<Meter | undefined>();
  let sensors = $state<Device[]>([]);
  let attention = $state<Item[]>([]);
  let now = $state(Math.floor(Date.now() / 1000));
  let pageState = $state<"loading" | "error" | "success">("loading");
  let naming = $state(false);
  let name = $state("");
  let nameError = $state("");
  let controller: AbortController | undefined;

  async function load() {
    controller?.abort();
    controller = new AbortController();
    try {
      const [current, devices, asking] = await Promise.all([
        api.meter(controller.signal),
        api.devices(controller.signal),
        api.attention(controller.signal),
      ]);
      meter = current;
      sensors = devices.filter((d) => d.kind === "sensor" && !d.ignored);
      attention = asking.items.filter(
        (item) => attentionHref(item) === "/meter",
      );
      now = Math.floor(Date.now() / 1000);
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

  function startNaming() {
    name = meter?.name ?? "";
    nameError = "";
    naming = true;
  }

  async function saveName() {
    nameError = deviceNameError(name) ?? "";
    if (nameError) return;
    try {
      // Blank gives the meter its default name back.
      meter = await api.nameMeter(name.trim() || null);
      naming = false;
    } catch (e) {
      nameError = (e as Error).message;
    }
  }

  const status = $derived(meter ? meterState(meter) : undefined);
  const adapter = $derived(meter ? adapterText(meter) : null);
  // The latest the server powered the adapter on, or off and on.
  const mended = $derived(
    Math.max(meter?.powered_on_at ?? 0, meter?.power_cycled_at ?? 0) || null,
  );
  const value = (n: number | null | undefined, digits: number) =>
    n == null ? "—" : n.toFixed(digits);
</script>

<div class="sub-header"><h1>温度計</h1></div>

<section class="content" data-state={pageState}>
  <Attention items={attention} {now} />
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
  {:else if status === "unset" && sensors.length === 0}
    <p class="state">温度計がありません</p>
  {:else}
    {#each sensors as sensor (sensor.id)}
      <section class="meter" aria-labelledby={`sensor-${sensor.id}`}>
        <h2 id={`sensor-${sensor.id}`}>
          <a href={`/devices/${sensor.id}`}>{deviceName(sensor)}</a>
          {#if sensor.state === "no_response"}
            <span class="badge danger">応答なし</span>
          {/if}
          {#if sensor.on === false}
            <span class="badge">自動運転 切</span>
          {/if}
        </h2>
        <dl class="readings">
          <div class="reading">
            <dt>温度</dt>
            <dd>{value(sensor.climate?.temperature, 1)}<span>℃</span></dd>
          </div>
          <div class="reading">
            <dt>湿度</dt>
            <dd>{value(sensor.climate?.humidity, 0)}<span>%</span></dd>
          </div>
          <div class="reading">
            <dt>電池</dt>
            <dd>{value(sensor.climate?.battery, 0)}<span>%</span></dd>
          </div>
        </dl>
      </section>
    {/each}

    {#if meter && status !== "unset"}
      <section class="meter" aria-labelledby="switchbot">
        {#if naming}
          <div class="field">
            <label for="meter-name">名前</label>
            <input
              id="meter-name"
              bind:value={name}
              maxlength="100"
              placeholder="SwitchBot"
              aria-invalid={nameError ? "true" : undefined}
              onchange={() => void saveName()}
              onkeydown={(e) => {
                if (e.key === "Escape") naming = false;
              }}
            />
            {#if nameError}<p class="field-error" role="alert">
                {nameError}
              </p>{/if}
          </div>
        {:else}
          <h2 id="switchbot">
            {meter.name ?? "SwitchBot"}
            <button
              class="icon-btn"
              type="button"
              aria-label="名前を変える"
              onclick={startNaming}
            >
              <Icon name="pencil" />
            </button>
          </h2>
        {/if}
        {#if status === "waiting" || (status === "stale" && meter.received_at)}
          <div class="state-wrap">
            {#if status === "waiting"}
              <p class="state">まだ受信していません</p>
            {:else if meter.received_at}
              <p class="state error">受信が途絶えています</p>
              <p class="note">
                最後の受信 {receivedTime(meter.received_at, true)}
              </p>
            {/if}
            {#if adapter}
              <p class="note" data-testid="adapter">{adapter}</p>
            {/if}
            {#if mended}
              <p class="note">電源の入れ直し {receivedTime(mended, true)}</p>
            {/if}
          </div>
        {:else if meter.reading && meter.received_at}
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
          <p class="note received">
            {receivedTime(meter.received_at, false)} 受信
          </p>
        {/if}
      </section>
    {/if}
  {/if}
</section>

<style lang="sass">
  .meter + .meter
    margin-top: var(--sp-5)

  h2
    display: flex
    align-items: center
    flex-wrap: wrap
    gap: var(--sp-1) var(--sp-2)
    min-height: 36px
    margin: 0 0 var(--sp-2)
    font-size: var(--fs-md)
    font-weight: 500
    overflow-wrap: anywhere

    a
      color: inherit

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
