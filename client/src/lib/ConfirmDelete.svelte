<script lang="ts">
  import Modal from "./Modal.svelte";

  let {
    name,
    ondelete,
    onclose,
  }: { name: string; ondelete: () => Promise<void>; onclose: () => void } =
    $props();

  let busy = $state(false);
  let error = $state("");

  async function remove() {
    busy = true;
    error = "";
    try {
      await ondelete();
    } catch (e) {
      error = (e as Error).message;
      busy = false;
    }
  }
</script>

<Modal title={`「${name}」を削除`} {onclose}>
  {#if error}<p class="error-banner" role="alert">{error}</p>{/if}
  <div class="actions">
    <button class="btn" type="button" onclick={onclose}>キャンセル</button>
    <button
      class="btn btn-primary"
      type="button"
      disabled={busy}
      onclick={remove}
      data-autofocus
    >
      削除
    </button>
  </div>
</Modal>

<style lang="sass">
  .actions
    display: flex
    justify-content: flex-end
    gap: var(--sp-2)
</style>
