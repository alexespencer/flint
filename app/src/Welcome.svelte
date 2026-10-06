<script lang="ts">
  import { api, errorKind, errorMessage, isAndroid, pickFolder } from "./lib/api";

  import type { StoreInfo } from "./lib/api";

  let {
    stores,
    ondone,
    oncancel,
  }: { stores: StoreInfo[]; ondone: () => void; oncancel: () => void } = $props();
  const hasStores = $derived(stores.length > 0);

  let mode = $state<"create" | "join">("create");
  let name = $state("Notes");
  let folder = $state("");
  let folderEdited = $state(false);
  let nameEdited = $state(false);
  let bucket = $state("");
  let prefix = $state("");
  let prefixEdited = $state(false);
  let region = $state("auto");
  let endpoint = $state("");
  let accessKeyId = $state("");
  let s3Secret = $state("");
  /** The store whose saved S3 account (endpoint, region, key ID and secret key) is being reused. */
  let reuseFrom = $state<string | null>(null);

  /** One entry per distinct S3 account among the stores already here. */
  const accounts = $derived(
    stores.filter((s, i) => stores.findIndex((o) => o.accessKeyId === s.accessKeyId && o.endpoint === s.endpoint) === i),
  );

  function useAccount(id: string) {
    const from = stores.find((s) => s.id === id);
    reuseFrom = from?.id ?? null;
    if (!from) return;
    endpoint = from.endpoint ?? "";
    region = from.region;
    accessKeyId = from.accessKeyId;
    bucket = bucket || from.bucket;
    s3Secret = "";
  }

  // Changing the key ID means it is no longer the saved account.
  function keyIdEdited() {
    reuseFrom = null;
  }
  let password = $state("");
  let repeat = $state("");
  let busy = $state("");
  let error = $state("");


  // Joining: what identifies a store on S3 is its folder in the bucket, so until a name is typed the name follows it
  // (so a store you remove and join again usually comes back under the name it had).
  $effect(() => {
    if (mode === "join" && prefixEdited && !nameEdited && prefix.trim()) name = prefix.trim();
  });

  // Until the user types their own, the folder and prefix follow the store's name.
  $effect(() => {
    const current = name;
    if (!folderEdited) api.defaultFolder(current.trim() || "Notes").then((f) => !folderEdited && (folder = f ?? ""));
    if (!prefixEdited) prefix = current.trim();
  });

  // An empty password is allowed: the store is still encrypted, but with a key anyone can derive.
  const ready = $derived(
    name.trim() && folder && bucket.trim() && accessKeyId.trim() && (s3Secret || reuseFrom) && (mode === "join" || password === repeat),
  );
  const noPassword = $derived(mode === "create" && password === "" && repeat === "");

  async function choose() {
    const picked = await pickFolder();
    if (picked) {
      folder = picked;
      folderEdited = true;
    }
  }

  async function submit(event: Event) {
    event.preventDefault();
    error = "";
    busy = mode === "create" ? "Creating…" : "Joining…";
    const args = {
      name: name.trim(),
      folder,
      bucket: bucket.trim(),
      prefix: prefix.trim(),
      region: region.trim() || "auto",
      endpoint: endpoint.trim() || null,
      accessKeyId: accessKeyId.trim(),
      s3Secret: reuseFrom ? "" : s3Secret,
      reuseSecretFrom: reuseFrom,
      password,
    };
    try {
      const id = mode === "create" ? await api.createStore(args) : await api.joinStore(args);
      busy = "First sync…";
      await api.sync(id).catch(() => undefined); // problems show up in the app, with a retry
      s3Secret = password = repeat = "";
      reuseFrom = null;
      ondone();
    } catch (e) {
      error = errorMessage(e);
      if (errorKind(e) === "storeExists") mode = "join";
      if (errorKind(e) === "noSuchStore") mode = "create";
    } finally {
      busy = "";
    }
  }
</script>

<main>
  <form onsubmit={submit}>
    <h1>{hasStores ? "Add a store" : "Welcome to Flint"}</h1>
    <div class="tabs" role="tablist">
      <button type="button" role="tab" aria-selected={mode === "create"} class:on={mode === "create"} onclick={() => (mode = "create")}>
        New store
      </button>
      <button type="button" role="tab" aria-selected={mode === "join"} class:on={mode === "join"} onclick={() => (mode = "join")}>
        Join existing
      </button>
    </div>
    <p class="dim">
      {#if mode === "create"}
        A store is a folder of notes, encrypted with its own password and synced to a bucket you own.{#if !isAndroid} Notes you drop into the folder are encrypted in place.{/if}
      {:else}
        Add a store you made on another device. You need its bucket details and its password.
      {/if}
    </p>

    <label>Name <input bind:value={name} oninput={() => (nameEdited = true)} required /></label>
    {#if !isAndroid}
      <label>
        Folder on this Mac
        <span class="pick">
          <input bind:value={folder} oninput={() => (folderEdited = true)} required />
          <button type="button" onclick={choose}>Choose…</button>
        </span>
      </label>
    {/if}
    <fieldset>
      <legend>S3</legend>
      {#if accounts.length > 0}
        <label>
          Account
          <select value={reuseFrom ?? ""} onchange={(e) => (e.currentTarget.value ? useAccount(e.currentTarget.value) : (reuseFrom = null))}>
            <option value="">Enter new details</option>
            {#each accounts as a (a.id)}
              <option value={a.id}>{a.accessKeyId} · {a.endpoint ?? "AWS"}</option>
            {/each}
          </select>
        </label>
      {/if}
      <label>Bucket <input bind:value={bucket} required /></label>
      <label>Folder in the bucket <input bind:value={prefix} oninput={() => (prefixEdited = true)} /></label>
      <label>Endpoint <input bind:value={endpoint} placeholder="https://… (leave empty for AWS)" /></label>
      <label>Region <input bind:value={region} /></label>
      <label>Access key ID <input bind:value={accessKeyId} oninput={keyIdEdited} required autocomplete="off" /></label>
      {#if reuseFrom}
        <p class="dim">Using the secret access key already saved on this Mac for this account.</p>
      {:else}
        <label>Secret access key <input type="password" bind:value={s3Secret} required autocomplete="off" /></label>
      {/if}
    </fieldset>
    <label>
      {mode === "create" ? "New password (optional)" : "Password (leave empty if the store has none)"}
      <input type="password" bind:value={password} autocomplete="new-password" />
    </label>
    {#if mode === "create"}
      <label>Repeat password <input type="password" bind:value={repeat} autocomplete="new-password" /></label>
      {#if repeat !== password}<p class="error">The passwords differ.</p>{/if}
      {#if noPassword}
        <p class="warning">
          <strong>No password set.</strong> Your notes are still encrypted, but with a key anyone can work out, so
          whoever can read your bucket or this folder can read your notes. A password can be set later.
        </p>
      {:else}
        <p class="dim">There is no way to recover a forgotten password: nobody, including Flint, can read your notes without it.</p>
      {/if}
    {/if}

    {#if error}<p class="error">{error}</p>{/if}
    <div class="row">
      <button class="primary" disabled={!ready || !!busy}>{busy || (mode === "create" ? "Create store" : "Join store")}</button>
      {#if hasStores}<button type="button" onclick={oncancel} disabled={!!busy}>Cancel</button>{/if}
    </div>
  </form>
</main>

<style>
  main {
    height: 100%;
    overflow-y: auto;
    display: flex;
    justify-content: center;
    padding: 28px 16px;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 440px;
    height: max-content;
  }
  h1 {
    margin: 0;
    font-size: 22px;
  }
  p {
    margin: 0;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 12px;
    color: var(--fg-dim);
  }
  label input {
    font-size: 14px;
    color: var(--fg);
  }
  fieldset {
    display: flex;
    flex-direction: column;
    gap: 8px;
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  legend {
    color: var(--fg-dim);
    font-size: 12px;
  }
  .pick {
    display: flex;
    gap: 6px;
  }
  .tabs {
    display: flex;
    gap: 6px;
  }
  .tabs button.on {
    background: var(--bg-active);
    font-weight: 600;
  }
  .row {
    display: flex;
    gap: 8px;
  }
  .warning {
    border: 1px solid var(--danger);
    border-radius: 8px;
    padding: 8px 10px;
    font-size: 13px;
    color: var(--danger);
  }
</style>
