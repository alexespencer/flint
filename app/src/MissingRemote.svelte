<script lang="ts">
  import { api, errorMessage, type StoreInfo } from "./lib/api";
  import { backLayer } from "./lib/layers";

  let {
    store,
    onclose,
    onrestored,
    onremoved,
  }: { store: StoreInfo; onclose: () => void; onrestored: () => void; onremoved: () => void } = $props();

  $effect(() => backLayer(() => onclose())); // not tracked: the effect runs once, when the dialog opens

  let busy = $state("");
  let error = $state("");

  async function reupload() {
    busy = "Putting this device's copy back on S3…";
    error = "";
    try {
      await api.reuploadStore(store.id);
      onrestored();
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = "";
    }
  }

  async function remove() {
    if (!confirm(`Remove “${store.name}” from this device?\\n\\nIts folder is renamed (${store.name}.removed-…), not deleted, so the notes stay on this device in that folder.`)) return;
    busy = "Removing…";
    error = "";
    try {
      await api.removeStore(store.id, true); // it cannot be synced, so there is nothing to check first
      onremoved();
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = "";
    }
  }
</script>

<div class="backdrop" role="presentation">
  <div class="dialog" role="alertdialog" aria-label="Store missing from S3" tabindex="-1">
    <h2>“{store.name}” is no longer on S3</h2>
    <p>
      It was deleted there, or the bucket was emptied. <strong>Nothing was changed on this device</strong>: your notes and
      the key are all still here.
    </p>
    <p class="dim">
      <strong>Put this device's copy back</strong> uploads everything here to S3 again, and other devices pick the store
      up where it was. <strong>Remove from this device</strong> keeps the folder (renamed) and forgets the store.
    </p>
    {#if error}<p class="error">{error}</p>{/if}
    <div class="buttons">
      <button onclick={onclose} disabled={!!busy}>Not now</button>
      <button onclick={remove} disabled={!!busy}>Remove from this device</button>
      <button class="primary" onclick={reupload} disabled={!!busy}>{busy || "Put this device's copy back"}</button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.5);
    display: grid;
    place-items: center;
    z-index: 40;
  }
  .dialog {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 12px;
    width: min(520px, 92vw);
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h2 {
    margin: 0;
    font-size: 17px;
  }
  p {
    margin: 0;
  }
  .buttons {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
</style>
