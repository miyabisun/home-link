<script lang="ts">
  import { type Attention, attentionHref, attentionText } from "./admin";

  // What asks for the user, each a link to the page that shows more of it.
  let { items, now }: { items: Attention[]; now: number } = $props();
</script>

{#if items.length}
  <ul class="attention" aria-label="要対応">
    {#each items as item, i (i)}
      <li>
        <a class="error-banner" href={attentionHref(item)}
          >{attentionText(item, now)}</a
        >
      </li>
    {/each}
  </ul>
{/if}

<style lang="sass">
  .attention
    display: flex
    flex-direction: column
    gap: var(--sp-1)
    margin: 0 0 var(--sp-3)
    padding: 0
    list-style: none

  a
    display: flex
    align-items: center
    box-sizing: border-box
    min-height: 44px
    margin: 0
    text-decoration: none
    overflow-wrap: anywhere

    &:hover
      text-decoration: underline
</style>
