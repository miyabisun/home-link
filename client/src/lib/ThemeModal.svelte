<script lang="ts">
  import Icon from "./Icon.svelte";
  import Modal from "./Modal.svelte";
  import { loadTheme, saveTheme, type ThemeChoice } from "./theme";

  let { onclose }: { onclose: () => void } = $props();

  let choice = $state<ThemeChoice>(loadTheme());

  const options: { value: ThemeChoice; label: string; icon: string }[] = [
    { value: "system", label: "自動", icon: "monitor" },
    { value: "light", label: "ライト", icon: "sun" },
    { value: "dark", label: "ダーク", icon: "moon" },
  ];

  // 選択してもモーダルは閉じない: テーマの変化をその場で目視確認させる
  // (DESIGN.md, Theme settings modal)。
  function choose(value: ThemeChoice) {
    choice = value;
    saveTheme(value);
  }
</script>

<Modal title="テーマ設定" {onclose}>
  <div class="options" role="radiogroup" aria-label="テーマ">
    {#each options as option (option.value)}
      <button
        class="option"
        class:selected={choice === option.value}
        type="button"
        role="radio"
        aria-checked={choice === option.value}
        data-autofocus={choice === option.value ? "" : undefined}
        onclick={() => choose(option.value)}
      >
        <Icon name={option.icon} />
        <span>{option.label}</span>
      </button>
    {/each}
  </div>
</Modal>

<style lang="sass">
  .options
    display: flex
    flex-direction: column
    gap: var(--sp-2)

  .option
    display: flex
    align-items: center
    gap: var(--sp-2)
    min-height: 44px
    padding: var(--sp-2) var(--sp-3)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-sm)
    background: var(--c-surface-raised)
    color: var(--c-on-surface)
    font-size: var(--fs-md)
    font-weight: 500
    cursor: pointer

    &:hover
      background: var(--c-hover-1)

    &.selected
      border-color: var(--c-accent)
      background: var(--c-accent-subtle)
</style>
