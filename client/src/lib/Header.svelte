<script lang="ts">
  import Icon from "./Icon.svelte";
  import ThemeModal from "./ThemeModal.svelte";

  let menuOpen = $state(false);
  let themeOpen = $state(false);
  let menuButton = $state<HTMLButtonElement | undefined>();

  function closeMenu() {
    menuOpen = false;
    menuButton?.focus();
  }

  function openTheme() {
    menuOpen = false;
    themeOpen = true;
  }

  function closeTheme() {
    themeOpen = false;
    menuButton?.focus();
  }

  function onkeydown(event: KeyboardEvent) {
    if (menuOpen && event.key === "Escape") {
      event.preventDefault();
      closeMenu();
    }
  }
</script>

<svelte:window {onkeydown} />

<header>
  <a class="title" href="/">home-link</a>
  <div class="menu-wrapper">
    <button
      class="icon-btn"
      type="button"
      aria-label="メニュー"
      aria-expanded={menuOpen}
      bind:this={menuButton}
      onclick={() => (menuOpen = !menuOpen)}
    >
      <Icon name="menu" />
    </button>
    {#if menuOpen}
      <button
        class="menu-overlay"
        type="button"
        tabindex="-1"
        aria-label="メニューを閉じる"
        onclick={closeMenu}
      ></button>
      <nav class="menu">
        <button class="menu-item" type="button" onclick={openTheme}>
          テーマ設定
        </button>
        <a class="menu-item" href="/labels" onclick={() => (menuOpen = false)}>
          ラベル
        </a>
        <a class="menu-item" href="/meter" onclick={() => (menuOpen = false)}>
          温度計
        </a>
      </nav>
    {/if}
  </div>
</header>

{#if themeOpen}
  <ThemeModal onclose={closeTheme} />
{/if}

<style lang="sass">
  header
    position: sticky
    top: 0
    z-index: 10
    display: flex
    align-items: center
    justify-content: space-between
    height: var(--header-h)
    padding: 0 var(--sp-3)
    background: var(--c-wash-base)
    border-bottom: 1px solid var(--c-border)

  .title
    font-size: var(--fs-md)
    font-weight: 500
    color: var(--c-on-surface)
    text-decoration: none

  .menu-wrapper
    position: relative
    display: flex
    align-items: center
    align-self: stretch

  .menu-overlay
    position: fixed
    inset: 0
    z-index: 19
    padding: 0
    border: none
    background: transparent
    cursor: default

  .menu
    position: absolute
    top: 100%
    right: 0
    z-index: 20
    display: flex
    flex-direction: column
    min-width: 180px
    overflow: hidden
    border: 1px solid var(--c-border)
    border-radius: var(--radius-lg)
    background: var(--c-surface-raised)
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.25)

  .menu-item
    display: block
    width: 100%
    padding: var(--sp-2) var(--sp-3)
    border: none
    background: transparent
    color: var(--c-on-surface)
    font-size: var(--fs-md)
    font-weight: 500
    text-align: left
    text-decoration: none
    cursor: pointer

    &:hover
      background: var(--c-hover-1)
</style>
