<script lang="ts">
  import { api, errorMessage, type Conflict } from "./lib/api";
  import Editor from "./Editor.svelte";

  let {
    storeId,
    conflict,
    onclose,
    onresolved,
  }: { storeId: string; conflict: Conflict; onclose: () => void; onresolved: () => void } = $props();

  let mine = $state<string | null>(null);
  let theirs = $state<string | null>(null);
  let error = $state("");
  let busy = $state(false);

  $effect(() => {
    mine = theirs = null;
    api.readNote(storeId, conflict.path).then((t) => (mine = t), (e) => (error = errorMessage(e)));
    if (conflict.copy) {
      api.readNote(storeId, conflict.copy).then((t) => (theirs = t), (e) => (error = errorMessage(e)));
    } else {
      theirs = "";
    }
  });

  async function resolve() {
    busy = true;
    error = "";
    try {
      await api.resolve(storeId, conflict.path);
      onresolved();
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = false;
    }
  }

  async function useTheirs() {
    if (theirs === null) return;
    await api.saveNote(storeId, conflict.path, theirs);
    mine = theirs;
  }
</script>

<div class="backdrop" role="presentation">
  <div class="dialog" role="dialog" aria-label="Resolve conflict">
    <header>
      <strong>{conflict.path}</strong>
      <span class="dim">
        {conflict.copy ? "Changed on two devices. Edit the left side until it is right, then resolve." : "Edited here, deleted on another device."}
      </span>
    </header>
    <div class="panes">
      <section>
        <h3>This device (editable)</h3>
        {#if mine !== null}
          {#key mine}
            <Editor text={mine} onchange={(t) => api.saveNote(storeId, conflict.path, t)} />
          {/key}
        {/if}
      </section>
      <section>
        <h3>{conflict.copy ? `The other version (${conflict.copy})` : "Other device"}</h3>
        {#if theirs !== null}
          {#if conflict.copy}
            <Editor text={theirs} readonly />
          {:else}
            <p class="dim pad">Nothing: it was deleted there.</p>
          {/if}
        {/if}
      </section>
    </div>
    {#if error}<p class="error pad">{error}</p>{/if}
    <footer>
      {#if conflict.copy}<button onclick={useTheirs} disabled={theirs === null}>Use the other version</button>{/if}
      <span class="spacer"></span>
      <button onclick={onclose}>Close</button>
      <button class="primary" onclick={resolve} disabled={busy}>Mark as resolved</button>
    </footer>
    <p class="dim pad hint">
      Resolving keeps what is on the left. The other version's file is moved to the folder's .trash, here and on your other devices.
    </p>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.45);
    display: grid;
    place-items: center;
    z-index: 10;
  }
  .dialog {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
    width: min(1100px, 94vw);
    height: min(720px, 90vh);
    display: flex;
    flex-direction: column;
  }
  header,
  footer {
    display: flex;
    gap: 12px;
    align-items: baseline;
    padding: 10px 14px;
  }
  footer {
    align-items: center;
  }
  .spacer {
    flex: 1;
  }
  .panes {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 1fr 1fr;
    border-block: 1px solid var(--border);
  }
  section {
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  section + section {
    border-left: 1px solid var(--border);
    background: var(--bg-side);
  }
  h3 {
    margin: 0;
    padding: 8px 14px;
    font-size: 12px;
    color: var(--fg-dim);
    font-weight: 500;
  }
  section :global(.editor) {
    flex: 1;
    min-height: 0;
  }
  .pad {
    padding: 0 14px;
  }
  .hint {
    font-size: 12px;
    padding-bottom: 10px;
    margin: 0;
  }
</style>
