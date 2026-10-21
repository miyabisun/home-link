<script lang="ts">
  import ConfirmDelete from "../lib/ConfirmDelete.svelte";
  import Icon from "../lib/Icon.svelte";
  import {
    deviceName,
    dimming,
    dimmingChange,
    labelNameError,
    overrideInput,
    percentOf,
    scheduleInput,
    switchProblem,
    withLabel,
    type Dimming,
  } from "../lib/admin";
  import { aborted, api, ApiError, type Device, type Label } from "../lib/api";
  import { navigate } from "../lib/router.svelte";

  let { id }: { id: string } = $props();

  let label = $state<Label | undefined>();
  let devices = $state<Device[]>([]);
  let pageState = $state<"loading" | "error" | "missing" | "success">(
    "loading",
  );
  let name = $state("");
  // Number inputs bind numbers, or null when empty.
  let percent = $state<number | null>(null);
  let kelvin = $state<number | null>(null);
  let rows = $state<{ key: number; time: string; on: boolean }[]>([]);
  let nextKey = 0;
  let nameError = $state("");
  let overrideError = $state("");
  let overrideField = $state<"percent" | "kelvin" | "">("");
  let scheduleError = $state("");
  let error = $state("");
  let switching = $state(false);
  let confirming = $state(false);
  let controller: AbortController | undefined;

  function show(next: Label) {
    label = next;
    name = next.name;
    percent =
      next.override_level === null ? null : percentOf(next.override_level);
    kelvin = next.override_kelvin;
    rows = next.schedule.map((s) => ({ key: nextKey++, ...s }));
  }

  async function load() {
    controller?.abort();
    controller = new AbortController();
    pageState = "loading";
    try {
      const [all, ledger] = await Promise.all([
        api.labels(controller.signal),
        api.devices(controller.signal),
      ]);
      devices = ledger;
      const found = all.find((l) => String(l.id) === id);
      if (found) show(found);
      pageState = found ? "success" : "missing";
    } catch (e) {
      if (!aborted(e)) pageState = "error";
    }
  }

  $effect(() => {
    void load();
    return () => controller?.abort();
  });

  /** Sends `patch`; the field's own error takes a rejection, else the banner. */
  async function save(
    patch: Parameters<typeof api.updateLabel>[1],
    onError: (message: string) => void = (m) => (error = m),
  ) {
    if (!label) return;
    error = "";
    try {
      const saved = await api.updateLabel(label.id, patch);
      label = { ...saved };
    } catch (e) {
      onError((e as Error).message);
    }
  }

  function saveName() {
    nameError = labelNameError(name) ?? "";
    if (!nameError && label && name.trim() !== label.name) {
      void save({ name: name.trim() }, (m) => (nameError = m));
    }
  }

  async function chooseDimming(mode: Dimming) {
    if (!label) return;
    overrideError = "";
    await save(dimmingChange(mode, label));
    if (label) show({ ...label, schedule: label.schedule });
  }

  function saveOverride() {
    const input = overrideInput(String(percent ?? ""), String(kelvin ?? ""));
    if ("error" in input) {
      overrideError = input.error;
      overrideField = input.field;
      return;
    }
    overrideError = "";
    overrideField = "";
    void save(input.patch, (m) => (overrideError = m));
  }

  function saveSchedule() {
    const input = scheduleInput(rows);
    if ("error" in input) {
      scheduleError = input.error;
      return;
    }
    scheduleError = "";
    void save(input, (m) => (scheduleError = m));
  }

  function removeRow(key: number) {
    rows = rows.filter((row) => row.key !== key);
    saveSchedule();
  }

  async function toggle() {
    if (!label) return;
    switching = true;
    error = "";
    try {
      const result = await api.toggleLabel(label.id);
      label.on = result.on;
      error = switchProblem(result) ?? "";
    } catch (e) {
      error = (e as Error).message;
    }
    switching = false;
  }

  async function assign(device: Device, on: boolean) {
    if (!label) return;
    error = "";
    try {
      const saved = await api.updateDevice(device.id, {
        label_ids: withLabel(
          device.labels.map((l) => l.id),
          label.id,
          on,
        ),
      });
      devices = devices.map((d) => (d.id === saved.id ? saved : d));
    } catch (e) {
      error = (e as Error).message;
    }
  }

  async function remove() {
    if (!label) return;
    try {
      await api.deleteLabel(label.id);
    } catch (e) {
      if (!(e instanceof ApiError && e.status === 404)) throw e;
    }
    navigate("/labels");
  }
</script>

<div class="sub-header">
  <h1>{label ? label.name : "ラベル"}</h1>
</div>

<section class="content" data-state={pageState}>
  {#if pageState === "loading"}
    <p class="state">
      <span class="spinner" aria-hidden="true"></span>読み込み中…
    </p>
  {:else if pageState === "missing"}
    <p class="state">ラベルが見つかりません</p>
  {:else if pageState === "error"}
    <div class="state-wrap">
      <p class="state error">読み込みに失敗しました</p>
      <button class="btn" type="button" onclick={() => void load()}
        >再試行</button
      >
    </div>
  {:else if label}
    {#if error}<p class="error-banner" role="alert">{error}</p>{/if}

    <div class="field name-row">
      <div class="grow">
        <label for="label-name">名前</label>
        <input
          id="label-name"
          bind:value={name}
          maxlength="100"
          aria-invalid={nameError ? "true" : undefined}
          aria-describedby={nameError ? "label-name-error" : undefined}
          onchange={saveName}
        />
      </div>
      <button
        class="btn"
        type="button"
        aria-label="入り切り"
        aria-pressed={label.on === true}
        disabled={switching}
        onclick={toggle}
      >
        {label.on ? "入" : "切"}
      </button>
    </div>
    {#if nameError}<p
        class="field-error name-error"
        id="label-name-error"
        role="alert"
      >
        {nameError}
      </p>{/if}

    <div class="field">
      <label for="label-dimming">調光</label>
      <select
        id="label-dimming"
        value={dimming(label)}
        onchange={(e) => void chooseDimming(e.currentTarget.value as Dimming)}
      >
        <option value="none">なし</option>
        <option value="auto">自動調光</option>
        <option value="fixed">固定</option>
      </select>
    </div>

    {#if dimming(label) === "fixed"}
      <div class="field pair">
        <div>
          <label for="label-percent">明るさ（%）</label>
          <input
            id="label-percent"
            type="number"
            inputmode="numeric"
            min="1"
            max="100"
            bind:value={percent}
            aria-invalid={overrideField === "percent" ? "true" : undefined}
            aria-describedby={overrideError
              ? "label-override-error"
              : undefined}
            onchange={saveOverride}
          />
        </div>
        <div>
          <label for="label-kelvin">色温度（K）</label>
          <input
            id="label-kelvin"
            type="number"
            inputmode="numeric"
            min="1000"
            max="10000"
            step="100"
            bind:value={kelvin}
            aria-invalid={overrideField === "kelvin" ? "true" : undefined}
            aria-describedby={overrideError
              ? "label-override-error"
              : undefined}
            onchange={saveOverride}
          />
        </div>
        {#if overrideError}<p
            class="field-error"
            id="label-override-error"
            role="alert"
          >
            {overrideError}
          </p>{/if}
      </div>
    {/if}

    <fieldset class="field">
      <legend>スケジュール</legend>
      <ul class="rows">
        {#each rows as row, index (row.key)}
          <li>
            <input
              type="time"
              aria-label={`時刻${index + 1}`}
              bind:value={row.time}
              onchange={saveSchedule}
            />
            <select
              aria-label={`操作${index + 1}`}
              bind:value={row.on}
              onchange={saveSchedule}
            >
              <option value={true}>入</option>
              <option value={false}>切</option>
            </select>
            <button
              class="icon-btn"
              type="button"
              aria-label={`時刻${index + 1}を削除`}
              onclick={() => removeRow(row.key)}
            >
              <Icon name="trash" />
            </button>
          </li>
        {/each}
      </ul>
      {#if scheduleError}<p class="field-error" role="alert">
          {scheduleError}
        </p>{/if}
      <button
        class="btn"
        type="button"
        onclick={() =>
          (rows = [...rows, { key: nextKey++, time: "", on: true }])}
      >
        時刻を追加
      </button>
    </fieldset>

    <fieldset class="field">
      <legend>機器</legend>
      {#if devices.length === 0}
        <p class="note">機器がありません</p>
      {:else}
        <ul class="checks">
          {#each devices as device (device.id)}
            <li>
              <label>
                <input
                  type="checkbox"
                  checked={device.labels.some((l) => l.id === label!.id)}
                  onchange={(e) => void assign(device, e.currentTarget.checked)}
                />
                {deviceName(device)}
              </label>
            </li>
          {/each}
        </ul>
      {/if}
    </fieldset>

    <button
      class="btn btn-danger"
      type="button"
      onclick={() => (confirming = true)}>削除</button
    >
  {/if}
</section>

{#if confirming && label}
  <ConfirmDelete
    name={label.name}
    ondelete={remove}
    onclose={() => (confirming = false)}
  />
{/if}

<style lang="sass">
  .name-row
    display: flex
    align-items: flex-end
    gap: var(--sp-2)
    margin-bottom: var(--sp-4)

    .grow
      flex: 1
      min-width: 0

    .btn
      flex-shrink: 0
      min-width: 52px
      height: 44px

  .name-error
    margin-top: calc(var(--sp-3) * -1)
    margin-bottom: var(--sp-4)

  .pair
    display: grid
    grid-template-columns: 1fr 1fr
    gap: var(--sp-2)

    .field-error
      grid-column: 1 / -1

  .rows
    display: flex
    flex-direction: column
    gap: var(--sp-2)
    margin: 0 0 var(--sp-2)
    padding: 0
    list-style: none

    li
      display: grid
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr) 36px
      align-items: center
      gap: var(--sp-2)

  .note
    margin: 0
    color: var(--c-muted)
    font-size: var(--fs-sm)
</style>
