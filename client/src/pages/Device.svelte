<script lang="ts">
  import ConfirmDelete from "../lib/ConfirmDelete.svelte";
  import {
    deviceName,
    deviceNameError,
    stateText,
    withLabel,
  } from "../lib/admin";
  import { aborted, api, missing, type Device, type Label } from "../lib/api";
  import { navigate } from "../lib/router.svelte";

  let { id }: { id: string } = $props();

  let device = $state<Device | undefined>();
  let labels = $state<Label[]>([]);
  let pageState = $state<"loading" | "error" | "missing" | "success">(
    "loading",
  );
  let name = $state("");
  let nameError = $state("");
  let error = $state("");
  let switching = $state(false);
  let confirming = $state(false);
  let controller: AbortController | undefined;

  async function load() {
    controller?.abort();
    controller = new AbortController();
    pageState = "loading";
    try {
      [device, labels] = await Promise.all([
        api.device(id, controller.signal),
        api.labels(controller.signal),
      ]);
      name = device.name;
      pageState = "success";
    } catch (e) {
      if (!aborted(e)) pageState = missing(e) ? "missing" : "error";
    }
  }

  $effect(() => {
    void load();
    return () => controller?.abort();
  });

  async function save(patch: Parameters<typeof api.updateDevice>[1]) {
    if (!device) return;
    error = "";
    try {
      device = await api.updateDevice(device.id, patch);
    } catch (e) {
      error = (e as Error).message;
    }
  }

  function saveName() {
    nameError = deviceNameError(name) ?? "";
    if (!nameError && device && name.trim() !== device.name)
      void save({ name: name.trim() });
  }

  async function switchPlug() {
    if (!device) return;
    switching = true;
    error = "";
    try {
      const result = await api.switchDevice(device.id, !device.on);
      if (result.state === "no_response") error = "応答なし";
      else device.on = result.state === "on";
    } catch (e) {
      error = (e as Error).message;
    }
    switching = false;
  }

  async function remove() {
    if (!device) return;
    await api.deleteDevice(device.id);
    navigate("/");
  }
</script>

<div class="sub-header">
  <h1>{device ? deviceName(device) : "機器"}</h1>
</div>

<section class="content" data-state={pageState}>
  {#if pageState === "loading"}
    <p class="state">
      <span class="spinner" aria-hidden="true"></span>読み込み中…
    </p>
  {:else if pageState === "missing"}
    <p class="state">機器が見つかりません</p>
  {:else if pageState === "error"}
    <div class="state-wrap">
      <p class="state error">読み込みに失敗しました</p>
      <button class="btn" type="button" onclick={() => void load()}
        >再試行</button
      >
    </div>
  {:else if device}
    {@const state = stateText(device.state, device.left_fabric)}
    {#if error}<p class="error-banner" role="alert">{error}</p>{/if}
    {@const details = [device.product_name, device.wifi_ssid].filter(
      (text) => text && text !== deviceName(device!),
    )}
    {#if state || details.length}
      <p class="meta">
        {#if state}
          <span
            class="badge"
            class:danger={device.state === "no_response" || device.left_fabric}
            >{state}</span
          >
        {/if}
        {#each details as text (text)}
          <span>{text}</span>
        {/each}
      </p>
    {/if}

    <div class="field name-row">
      <div class="grow">
        <label for="device-name">名前</label>
        <input
          id="device-name"
          bind:value={name}
          maxlength="100"
          placeholder={device.product_name ?? device.vendor ?? ""}
          aria-invalid={nameError ? "true" : undefined}
          aria-describedby={nameError ? "device-name-error" : undefined}
          onchange={saveName}
        />
      </div>
      {#if device.kind !== "light" && device.on !== null}
        <button
          class="btn"
          type="button"
          aria-label="入り切り"
          aria-pressed={device.on}
          disabled={switching}
          onclick={switchPlug}
        >
          {device.on ? "入" : "切"}
        </button>
      {/if}
    </div>
    {#if nameError}<p
        class="field-error name-error"
        id="device-name-error"
        role="alert"
      >
        {nameError}
      </p>{/if}

    <fieldset class="field">
      <legend>ラベル</legend>
      {#if labels.length === 0}
        <a href="/labels">ラベルを作成</a>
      {:else}
        <ul class="checks">
          {#each labels as label (label.id)}
            <li>
              <label>
                <input
                  type="checkbox"
                  checked={device.labels.some((l) => l.id === label.id)}
                  onchange={(e) =>
                    void save({
                      label_ids: withLabel(
                        device!.labels.map((l) => l.id),
                        label.id,
                        e.currentTarget.checked,
                      ),
                    })}
                />
                {label.name}
              </label>
            </li>
          {/each}
        </ul>
      {/if}
    </fieldset>

    <div class="field">
      <ul class="checks">
        <li>
          <label>
            <input
              type="checkbox"
              checked={device.ignored}
              onchange={(e) => void save({ ignored: e.currentTarget.checked })}
            />
            無視
          </label>
        </li>
      </ul>
    </div>

    <button
      class="btn btn-danger"
      type="button"
      onclick={() => (confirming = true)}>削除</button
    >
  {/if}
</section>

{#if confirming && device}
  <ConfirmDelete
    name={deviceName(device)}
    ondelete={remove}
    onclose={() => (confirming = false)}
  />
{/if}

<style lang="sass">
  .name-row
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

  .name-error
    margin: calc(var(--sp-3) * -1) 0 var(--sp-4)

  .meta
    display: flex
    flex-wrap: wrap
    align-items: center
    gap: var(--sp-1) var(--sp-2)
    margin: 0 0 var(--sp-4)
    color: var(--c-muted)
    font-size: var(--fs-sm)
    overflow-wrap: anywhere
</style>
