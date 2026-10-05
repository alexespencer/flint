<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { backLayer } from "./lib/layers";

  let {
    title,
    help,
    initial,
    confirmLabel,
    warn,
    reject,
    onconfirm,
    onclose,
  }: {
    title: string;
    help: string;
    initial: string;
    confirmLabel: string;
    /** A warning to show under the field for the value typed so far, if any. */
    warn?: (value: string) => string | null;
    /** A reason the value cannot be used; shown under the field, and the button is disabled. */
    reject?: (value: string) => string | null;
    onconfirm: (value: string) => void;
    onclose: () => void;
  } = $props();

  $effect(() => backLayer(() => onclose())); // not tracked: the effect runs once, when the layer opens

  let value = $state(untrack(() => initial)); // the starting text; typing replaces it
  let input: HTMLInputElement | undefined = $state();
  const warning = $derived(warn?.(value.trim()) ?? null);
  const rejection = $derived(reject?.(value.trim()) ?? null);
  const ready = $derived(value.trim() !== "" && value.trim() !== initial && !rejection);

  onMount(() => {
    // select the file's name, not its folder or its extension, so typing renames it
    const slash = initial.lastIndexOf("/") + 1;
    const dot = initial.lastIndexOf(".");
    input?.focus();
    input?.setSelectionRange(slash, dot > slash ? dot : initial.length);
  });

  function submit(event: Event) {
    event.preventDefault();
    if (ready) onconfirm(value.trim());
  }
</script>

<div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-label={title} tabindex="-1" onkeydown={(e) => e.key === "Escape" && onclose()}>
    <form onsubmit={submit}>
      <h2>{title}</h2>
      <p class="dim">{help}</p>
      <input bind:value bind:this={input} spellcheck="false" autocomplete="off" />
      {#if rejection}<p class="warn">{rejection}</p>{:else if warning}<p class="warn">{warning}</p>{/if}
      <div class="buttons">
        <button type="button" onclick={onclose}>Cancel</button>
        <button class="primary" disabled={!ready}>{confirmLabel}</button>
      </div>
    </form>
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
    padding: 16px 20px;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h2 {
    margin: 0;
    font-size: 16px;
  }
  p {
    margin: 0;
  }
  .warn {
    color: var(--danger);
    font-size: 13px;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
</style>
