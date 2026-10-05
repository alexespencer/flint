<script lang="ts">
  import { backLayer } from "./lib/layers";

  export interface MenuItem {
    label: string;
    danger?: boolean;
    /** Marks the current choice in a list of choices (a tick in front of the label). */
    checked?: boolean;
    /** A dividing line instead of an item (its label is not shown). */
    separator?: boolean;
    /** Shown dimmed and not clickable, to say why there is nothing to choose. */
    disabled?: boolean;
    action: () => void;
  }

  let { x, y, items, onclose }: { x: number; y: number; items: MenuItem[]; onclose: () => void } = $props();

  $effect(() => backLayer(() => onclose())); // not tracked: the effect runs once, when the layer opens

  let menu: HTMLUListElement | undefined = $state();
  let left = $state(0);
  let top = $state(0);

  // keep the menu on screen
  $effect(() => {
    const width = menu?.offsetWidth ?? 0;
    const height = menu?.offsetHeight ?? 0;
    left = Math.max(8, Math.min(x, window.innerWidth - width - 8));
    top = Math.max(8, Math.min(y, window.innerHeight - height - 8));
  });

  // when the menu is a list of choices, every label leaves room for the tick so they line up
  const hasTicks = $derived(items.some((item) => item.checked !== undefined));

  function choose(item: MenuItem) {
    onclose();
    item.action();
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} onresize={onclose} />

<div class="scrim" role="presentation" onmousedown={onclose} oncontextmenu={(e) => (e.preventDefault(), onclose())}></div>
<ul class="menu" role="menu" bind:this={menu} style:left="{left}px" style:top="{top}px">
  {#each items as item, i (i)}
    {#if item.separator}
      <li class="sep" role="separator"></li>
    {:else}
      <li role="none">
        <button role="menuitemradio" aria-checked={item.checked} class:danger={item.danger} disabled={item.disabled} onclick={() => choose(item)}>
          {#if hasTicks}<span class="tick">{item.checked ? "✓" : ""}</span>{/if}{item.label}
        </button>
      </li>
    {/if}
  {/each}
</ul>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 20;
  }
  .menu {
    position: fixed;
    z-index: 21;
    list-style: none;
    margin: 0;
    padding: 4px;
    min-width: 190px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 8px 28px rgb(0 0 0 / 0.28);
  }
  .menu button {
    display: block;
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    border-radius: 5px;
    padding: 6px 10px;
  }
  .menu button:hover {
    background: var(--bg-hover);
  }
  .tick {
    display: inline-block;
    width: 1.3em;
  }
  .sep {
    height: 1px;
    margin: 4px 6px;
    background: var(--border);
  }
  .menu button:disabled {
    color: var(--fg-dim);
    cursor: default;
  }
  .menu button:disabled:hover {
    background: none;
  }
  .menu button.danger {
    color: var(--danger);
  }
</style>
