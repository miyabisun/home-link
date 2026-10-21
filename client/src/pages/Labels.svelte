<script lang="ts">
  import { labelNameError, labelSummary } from "../lib/admin";
  import { aborted, api, type Label } from "../lib/api";
  import { navigate } from "../lib/router.svelte";

  let labels = $state<Label[]>([]);
  let listState = $state<"loading" | "empty" | "error" | "success">("loading");
  let name = $state("");
  let nameError = $state("");
  let creating = $state(false);
  let controller: AbortController | undefined;

  async function load() {
    controller?.abort();
    controller = new AbortController();
    try {
      labels = await api.labels(controller.signal);
      listState = labels.length === 0 ? "empty" : "success";
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

  async function create(event: SubmitEvent) {
    event.preventDefault();
    nameError = labelNameError(name) ?? "";
    if (nameError) return;
    creating = true;
    try {
      const label = await api.createLabel(name.trim());
      navigate(`/labels/${label.id}`);
    } catch (e) {
      nameError = (e as Error).message;
    }
    creating = false;
  }
</script>

<section class="content" data-state={listState}>
  <form class="add" onsubmit={create}>
    <div class="add-row">
      <input
        aria-label="ラベル名"
        placeholder="ラベル名"
        bind:value={name}
        maxlength="100"
        aria-invalid={nameError ? "true" : undefined}
        aria-describedby={nameError ? "label-name-error" : undefined}
      />
      <button class="btn btn-primary" type="submit" disabled={creating}
        >追加</button
      >
    </div>
    {#if nameError}<p class="field-error" id="label-name-error" role="alert">
        {nameError}
      </p>{/if}
  </form>

  {#if listState === "loading"}
    <p class="state">
      <span class="spinner" aria-hidden="true"></span>読み込み中…
    </p>
  {:else if listState === "empty"}
    <p class="state">ラベルがありません</p>
  {:else if listState === "error"}
    <div class="state-wrap">
      <p class="state error">読み込みに失敗しました</p>
      <button class="btn" type="button" onclick={() => void load()}
        >再試行</button
      >
    </div>
  {:else}
    <ul class="cards">
      {#each labels as label (label.id)}
        {@const summary = labelSummary(label)}
        <li>
          <a class="card" href={`/labels/${label.id}`}>
            <span class="name">{label.name}</span>
            {#if summary}<span class="note">{summary}</span>{/if}
            <span class="note">{label.device_count}台</span>
            {#if label.on}<span class="badge">入</span>{/if}
          </a>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style lang="sass">
  .add
    margin-bottom: var(--sp-4)

  .add-row
    display: flex
    gap: var(--sp-2)

    .btn
      flex-shrink: 0
</style>
