<script lang="ts">
  import { onMount } from "svelte";

  let {
    title,
    targets,
    onpick,
    onclose,
  }: {
    title: string;
    /** Folder paths, in the order to show them; `""` is the top level. */
    targets: string[];
    onpick: (folder: string) => void;
    onclose: () => void;
  } = $props();

  let query = $state("");
  let search: HTMLInputElement | undefined = $state();
  const shown = $derived(
    targets.filter((t) => (t || "top level").toLowerCase().includes(query.trim().toLowerCase())),
  );

  onMount(() => search?.focus());
</script>

<div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-label={title} tabindex="-1" onkeydown={(e) => e.key === "Escape" && onclose()}>
    <h2>{title}</h2>
    <input type="search" placeholder="Find a folder" bind:value={query} bind:this={search} aria-label="Find a folder" />
    <ul>
      {#each shown as target (target)}
        <li><button onclick={() => onpick(target)}>{target || "Top level"}</button></li>
      {/each}
      {#if shown.length === 0}<li class="dim none">No folder matches.</li>{/if}
    </ul>
    <div class="buttons"><button onclick={onclose}>Cancel</button></div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.45);
    display: grid;
    place-items: center;
    z-index: 30;
  }
  .dialog {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 12px;
    width: min(460px, 92vw);
    max-height: 80vh;
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h2 {
    margin: 0;
    font-size: 16px;
    word-break: break-all;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    min-height: 0;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  li button {
    display: block;
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    border-radius: 0;
    padding: 8px 12px;
    word-break: break-all;
  }
  li button:hover {
    background: var(--bg-hover);
  }
  .none {
    padding: 10px 12px;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
  }
</style>
