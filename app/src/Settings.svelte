<script lang="ts">
  import { untrack } from "svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import PathDialog from "./PathDialog.svelte";
  import { api, errorMessage, isAndroid, type BiometricState, type SettingsInfo, type StoreInfo } from "./lib/api";
  import { removeStoreFlow } from "./lib/storeActions";
  import { backLayer } from "./lib/layers";
  import { chooseTheme, themeState } from "./lib/themeState.svelte";
  import { AUTO_LOCK_CHOICES, THEME_CHOICES, autoLockMinutes, setAutoLockMinutes, type Theme } from "./lib/settings";

  let {
    store,
    stores,
    settings,
    onclose,
    onchange,
    onsettings,
  }: {
    store: StoreInfo;
    stores: StoreInfo[];
    /** The store's shared settings as they stand. */
    settings: SettingsInfo;
    onclose: () => void;
    onchange: () => void;
    /** Settings were changed here: the new ones. */
    onsettings: (changed: SettingsInfo) => void;
  } = $props();

  $effect(() => backLayer(() => onclose())); // not tracked: the effect runs once, when the layer opens

  const method = isAndroid ? "fingerprint" : "Touch ID";

  /** Which group of settings is showing: this store's, the app's (this device), or the list of stores. */
  let tab = $state<"store" | "app" | "stores">("store");
  let minutes = $state(autoLockMinutes());
  // what is typed in the two folder boxes: starts as the current settings, and is saved when a box is left
  let attachmentFolder = $state(untrack(() => settings.attachmentFolder));
  let newNoteFolder = $state(untrack(() => settings.newNoteFolder));
  let knownFolders = $state<string[]>([]);
  let folderProblem = $state("");

  $effect(() => {
    api.folders(store.id).then((list) => (knownFolders = list.map((f) => f.path)), () => (knownFolders = []));
  });

  /** Saves the folder settings (the store tidies them and refuses an unusable one); the boxes show what was kept. */
  async function saveFolders() {
    folderProblem = "";
    try {
      const saved = await api.setSettings(store.id, { attachmentFolder, newNoteFolder });
      attachmentFolder = saved.attachmentFolder;
      newNoteFolder = saved.newNoteFolder;
      onsettings(saved);
    } catch (e) {
      folderProblem = errorMessage(e);
      attachmentFolder = settings.attachmentFolder;
      newNoteFolder = settings.newNoteFolder;
    }
  }
  let version = $state("");
  $effect(() => {
    getVersion().then((v) => (version = v), () => (version = ""));
  });
  let biometric = $state<BiometricState | null>(null);
  let busy = $state(false);
  let error = $state("");
  let notice = $state("");
  let deleting = $state<StoreInfo | null>(null);

  $effect(() => {
    biometric = null;
    api.biometricState(store.id).then((state) => (biometric = state), () => (biometric = { available: false, enabled: false }));
  });

  function chooseMinutes(value: number) {
    minutes = value;
    setAutoLockMinutes(value);
  }

  async function toggleBiometric() {
    if (!biometric) return;
    busy = true;
    error = "";
    try {
      if (biometric.enabled) await api.biometricDisable(store.id);
      else await api.biometricEnable(store.id); // asks for the fingerprint to confirm
      biometric = await api.biometricState(store.id);
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = false;
    }
  }

  async function remove(target: StoreInfo) {
    error = notice = "";
    try {
      const result = await removeStoreFlow(target);
      if (!result) return;
      notice = result.keptAs ? `“${target.name}” was removed from this device. Its folder was kept as ${result.keptAs}.` : `“${target.name}” was removed from this device.`;
      onchange();
      if (target.id === store.id) onclose();
    } catch (e) {
      error = errorMessage(e);
    }
  }

  async function deleteEverywhere(target: StoreInfo) {
    deleting = null;
    error = notice = "";
    try {
      const count = await api.deleteStore(target.id, target.name);
      notice = `“${target.name}” was deleted: ${count} file${count === 1 ? "" : "s"} removed from S3.`;
      onchange();
      if (target.id === store.id) onclose();
    } catch (e) {
      error = errorMessage(e);
    }
  }
</script>

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-label="Settings">
    <header>
      <h2>Settings</h2>
      <button onclick={onclose}>Done</button>
    </header>

    <div class="tabs" role="tablist">
      <button role="tab" aria-selected={tab === "store"} class:on={tab === "store"} onclick={() => (tab = "store")}>Store</button>
      <button role="tab" aria-selected={tab === "app"} class:on={tab === "app"} onclick={() => (tab = "app")}>App</button>
      <button role="tab" aria-selected={tab === "stores"} class:on={tab === "stores"} onclick={() => (tab = "stores")}>Stores ({stores.length})</button>
    </div>

    <div class="body">
      {#if tab === "store"}
        <p class="which dim">Settings for “{store.name}” <span class="note">· synced to all your devices</span></p>
        {#if settings.newerVersion !== null}
          <p class="error">These come from a newer version of Flint, so they cannot be changed here. Update Flint on this device.</p>
        {/if}
        <label class="row">
          <span>New notes go in</span>
          <input list="known-folders" bind:value={newNoteFolder} onchange={saveFolders} placeholder="(top level)" disabled={settings.newerVersion !== null} />
        </label>
        <label class="row">
          <span>Pasted pictures go in</span>
          <input list="known-folders" bind:value={attachmentFolder} onchange={saveFolders} placeholder="(top level)" disabled={settings.newerVersion !== null} />
        </label>
        <datalist id="known-folders">
          {#each knownFolders as folder (folder)}<option value={folder}></option>{/each}
        </datalist>
        {#if folderProblem}<p class="error">{folderProblem}</p>{/if}

        <p class="group dim spaced">This device only</p>
        <div class="row" title={isAndroid ? "" : "One Touch ID unlocks every store that has it turned on."}>
          <span>Unlock with {method}</span>
          {#if store.passwordless}
            <span class="dim">No password</span>
          {:else if biometric === null}
            <span class="dim">…</span>
          {:else if !biometric.available}
            <span class="dim">Not set up on this device</span>
          {:else}
            <button onclick={toggleBiometric} disabled={busy}>{biometric.enabled ? "On: turn off" : "Off: turn on"}</button>
          {/if}
        </div>
      {:else if tab === "app"}
        <p class="which dim">Applies to this device only</p>
        <label class="row">
          <span>Theme</span>
          <select value={themeState.value} onchange={(e) => chooseTheme(e.currentTarget.value as Theme)}>
            {#each THEME_CHOICES as choice (choice.theme)}
              <option value={choice.theme}>{choice.label}</option>
            {/each}
          </select>
        </label>
        <label class="row">
          <span>Lock after being idle for</span>
          <select value={minutes} onchange={(e) => chooseMinutes(Number(e.currentTarget.value))}>
            {#each AUTO_LOCK_CHOICES as choice (choice.minutes)}
              <option value={choice.minutes}>{choice.label}</option>
            {/each}
          </select>
        </label>
        {#if isAndroid}<p class="dim small">The app also locks as soon as you leave it.</p>{/if}

        <p class="group dim spaced">About</p>
        <div class="row">
          <span>Version</span>
          <span class="dim">{version || "…"}</span>
        </div>
      {:else}
        <ul class="stores">
          {#each stores as s (s.id)}
            <li>
              <div class="info">
                <strong>{s.name}</strong>
                <span class="dim" title={s.endpoint ?? s.bucket}>
                  {s.bucket}{s.endpoint ? ` · ${s.endpoint.replace(/^https?:\/\//, "")}` : ""}
                </span>
                {#if !isAndroid}<span class="dim" title={s.folder}>{s.folder}</span>{/if}
              </div>
              <span class="actions">
                <button onclick={() => remove(s)} title="Take it off this device; it stays on S3">Remove</button>
                <button class="danger" onclick={() => (deleting = s)} title="Delete it from S3, for every device">Delete…</button>
              </span>
            </li>
          {/each}
        </ul>
      {/if}

      {#if notice}<p class="pad notice">{notice}</p>{/if}
      {#if error}<p class="error pad">{error}</p>{/if}
    </div>
  </div>
</div>

{#if deleting}
  {@const target = deleting}
  <PathDialog
    title={`Delete “${target.name}” everywhere`}
    help={`This deletes every file of “${target.name}” from S3, so no device can use it any more, and removes it from this device. It cannot be undone. Other devices keep their notes until you remove the store there. Type the store's name to confirm.`}
    initial=""
    confirmLabel="Delete from S3"
    reject={(value) => (value === target.name ? null : "Type the name exactly, including capitals.")}
    onconfirm={() => deleteEverywhere(target)}
    onclose={() => (deleting = null)}
  />
{/if}

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
    border-radius: 12px;
    width: min(560px, 94vw);
    max-height: 90vh;
    padding: 4px 20px 16px;
    display: flex;
    flex-direction: column;
    overflow-y: auto; /* only if even the fixed parts do not fit */
  }
  header {
    flex: none;
  }
  /* Rows are a fixed height (long lines are cut short), and four and a half show, so the half row hints that the list scrolls. */
  ul.stores {
    --row: 4.5rem;
    max-height: calc(var(--row) * 4.5);
    overflow-y: auto;
    border-bottom: 1px solid var(--border);
  }
  .tabs {
    display: flex;
    gap: 4px;
    padding: 2px;
    margin: 6px 0 12px;
    background: var(--bg-side);
    border-radius: 8px;
    flex: none;
  }
  .tabs button {
    flex: 1;
    background: none;
    border-color: transparent;
    padding: 4px 8px;
  }
  .tabs button.on {
    background: var(--bg);
    border-color: var(--border);
    font-weight: 600;
  }
  /* the same height whichever tab shows, so the dialog does not jump */
  .body {
    min-height: 15rem;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .body p {
    margin: 0;
  }
  .which {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .which .note {
    text-transform: none;
    letter-spacing: 0;
    font-size: 12px;
    opacity: 0.85;
  }
  .group {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .small {
    font-size: 12px;
  }
  /* a faint line before "This device only", to set it apart from the synced settings above it */
  .spaced {
    margin-top: 10px;
    padding-top: 14px;
    border-top: 1px solid var(--border);
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  h2 {
    font-size: 18px;
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
  }
  .row select {
    width: auto;
  }
  .row input {
    width: 55%;
    flex: none;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    height: var(--row);
    flex: none;
    border-top: 1px solid var(--border);
  }
  .info {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font-size: 13px;
    line-height: 1.4;
  }
  .info > * {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pad {
    margin-top: 12px;
  }
  .notice {
    font-size: 13px;
    color: var(--fg-dim);
    word-break: break-all;
  }
  .actions {
    display: flex;
    gap: 6px;
    flex: none;
  }
  button.danger {
    color: var(--danger);
  }
</style>
