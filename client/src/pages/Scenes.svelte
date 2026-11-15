<script lang="ts">
  import {
    registeredDay,
    sceneRunText,
    stepsNote,
    type SceneRun,
  } from "../lib/admin";
  import { aborted, api, type Phone, type Scenes } from "../lib/api";
  import ConfirmDelete from "../lib/ConfirmDelete.svelte";
  import Icon from "../lib/Icon.svelte";

  let scenes = $state<Scenes | undefined>();
  let phones = $state<Phone[]>([]);
  let pinned = $state(false);
  let pageState = $state<"loading" | "error" | "success">("loading");
  // The document being edited; the saved one stays in `scenes` until it is saved.
  let draft = $state("");
  let saving = $state(false);
  let yamlError = $state("");
  let running = $state<string | undefined>();
  let runs = $state<Record<string, SceneRun>>({});
  let error = $state("");
  let removing = $state<
    { name: string; remove: () => Promise<void> } | undefined
  >();
  let controller: AbortController | undefined;

  const dirty = $derived(scenes !== undefined && draft !== scenes.yaml);

  async function load() {
    controller?.abort();
    controller = new AbortController();
    const { signal } = controller;
    try {
      const [nextScenes, nextPhones, location] = await Promise.all([
        api.scenes(signal),
        api.phones(signal),
        api.location(signal),
      ]);
      // An edit under way is kept; the list follows what is saved.
      if (!dirty) draft = nextScenes.yaml;
      scenes = nextScenes;
      phones = nextPhones;
      pinned = location.pin !== null;
      pageState = "success";
    } catch (e) {
      if (!aborted(e) && scenes === undefined) pageState = "error";
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

  async function run(name: string) {
    running = name;
    error = "";
    try {
      runs[name] = await api.runScene(name);
    } catch (e) {
      error = (e as Error).message;
    }
    running = undefined;
  }

  async function save() {
    saving = true;
    yamlError = "";
    try {
      scenes = await api.saveScenes(draft);
      draft = scenes.yaml;
      // The results were of the steps as they were.
      runs = {};
    } catch (e) {
      yamlError = (e as Error).message;
    }
    saving = false;
  }

  function removePhone(phone: Phone) {
    removing = {
      name: phone.name,
      remove: async () => {
        await api.deletePhone(phone.id);
        phones = phones.filter((other) => other.id !== phone.id);
        removing = undefined;
      },
    };
  }

  function removePin() {
    removing = {
      name: "自宅の位置",
      remove: async () => {
        pinned = (await api.clearPin()).pin !== null;
        removing = undefined;
      },
    };
  }
</script>

<div class="sub-header"><h1>操作</h1></div>

<section class="content" data-state={pageState}>
  {#if error}<p class="error-banner" role="alert">{error}</p>{/if}
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
  {:else if scenes}
    {#if scenes.scenes.length === 0}
      <p class="none">操作がありません</p>
    {:else}
      <ul class="cards">
        {#each scenes.scenes as scene (scene.name)}
          {@const result = runs[scene.name]}
          <li class="card">
            <span class="name">{scene.name}</span>
            <button
              class="btn"
              type="button"
              aria-label={`${scene.name}を実行`}
              disabled={running === scene.name}
              onclick={() => void run(scene.name)}>実行</button
            >
            <span class="note steps">{stepsNote(scene.steps)}</span>
            {#if result}
              {@const told = sceneRunText(result, scene.steps)}
              <div class="result" aria-live="polite">
                {#if result.steps.length === 0}
                  <p class="told" class:problem={told.problem}>{told.text}</p>
                {:else}
                  <details open={result.failed > 0}>
                    <summary class="told" class:problem={told.problem}
                      >{told.text}</summary
                    >
                    <ul class="steps-done">
                      {#each result.steps as step, index (index)}
                        <li class:failed={step.result === "failed"}>
                          <code>{step.step}</code>
                          <span class="how"
                            >{step.result === "ok" ? "済み" : "失敗"}</span
                          >
                          {#if step.message}<span class="why"
                              >{step.message}</span
                            >{/if}
                        </li>
                      {/each}
                    </ul>
                  </details>
                {/if}
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}

    <details class="definition">
      <summary>定義</summary>
      <textarea
        aria-label="操作の定義（YAML）"
        spellcheck="false"
        autocapitalize="off"
        rows={draft.split("\n").length + 1}
        bind:value={draft}
        aria-invalid={yamlError ? "true" : undefined}
        aria-describedby={yamlError ? "scenes-yaml-error" : undefined}
      ></textarea>
      {#if yamlError}<p class="field-error" id="scenes-yaml-error" role="alert">
          {yamlError}
        </p>{/if}
      <div class="save">
        <button
          class="btn btn-primary"
          type="button"
          disabled={!dirty || saving}
          onclick={() => void save()}>保存</button
        >
      </div>
    </details>

    <h2>電話</h2>
    {#if phones.length === 0}
      <p class="none">未登録（アプリで登録）</p>
    {:else}
      <ul class="cards">
        {#each phones as phone (phone.id)}
          <li class="card row">
            <span class="name">{phone.name}</span>
            <span class="note">{registeredDay(phone.registered_at)}</span>
            <button
              class="icon-btn"
              type="button"
              aria-label={`${phone.name}を削除`}
              onclick={() => removePhone(phone)}
            >
              <Icon name="trash" />
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    <h2>自宅の位置</h2>
    {#if pinned}
      <ul class="cards">
        <li class="card row">
          <span class="name">登録済み</span>
          <button
            class="icon-btn"
            type="button"
            aria-label="自宅の位置を削除"
            onclick={removePin}
          >
            <Icon name="trash" />
          </button>
        </li>
      </ul>
    {:else}
      <p class="none">未登録（アプリで登録）</p>
    {/if}
  {/if}
</section>

{#if removing}
  <ConfirmDelete
    name={removing.name}
    ondelete={removing.remove}
    onclose={() => (removing = undefined)}
  />
{/if}

<style lang="sass">
  li.card
    align-items: center

    &:hover
      background: var(--c-surface-raised)

    .btn, .icon-btn
      flex-shrink: 0

  .row
    flex-wrap: nowrap

  .steps, .result
    flex-basis: 100%
    min-width: 0
    overflow-wrap: anywhere

  .told
    margin: 0
    font-size: var(--fs-sm)

    &.problem
      color: var(--c-danger)

  summary
    cursor: pointer

  .steps-done
    display: flex
    flex-direction: column
    gap: var(--sp-1)
    margin: var(--sp-2) 0 0
    padding: 0
    list-style: none
    font-size: var(--fs-sm)

    li
      display: flex
      flex-wrap: wrap
      gap: 0 var(--sp-2)

    code
      min-width: 0
      overflow-wrap: anywhere
      font-size: inherit

    .how
      color: var(--c-muted)

    .failed .how, .why
      color: var(--c-danger)

    .why
      flex-basis: 100%

  .definition
    margin-top: var(--sp-3)

    summary
      padding: var(--sp-3) 0
      color: var(--c-muted)
      font-size: var(--fs-sm)

  textarea
    display: block
    width: 100%
    padding: var(--sp-2)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-sm)
    background: var(--c-surface)
    color: var(--c-on-surface)
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace
    font-size: var(--fs-sm)
    line-height: 1.6
    overflow-wrap: anywhere
    resize: none
    field-sizing: content

    &:focus
      border-color: var(--c-accent)

    &[aria-invalid="true"]
      border-color: var(--c-danger)

  .save
    display: flex
    justify-content: flex-end
    margin-top: var(--sp-2)

  h2
    margin: var(--sp-5) 0 var(--sp-2)
    font-size: var(--fs-md)
    font-weight: 500

  .none
    margin: 0
    color: var(--c-muted)
    font-size: var(--fs-sm)
</style>
