<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { api, errorKind, errorMessage, summarise, type Hit, type StoreInfo } from "./lib/api";
  import { pieces } from "./lib/snippet";
  import { buildTree, isNote, title } from "./lib/tree";
  import ConflictView from "./ConflictView.svelte";
  import Editor from "./Editor.svelte";
  import FileTree from "./FileTree.svelte";

  let {
    store,
    stores,
    onchange,
    onswitch,
    onadd,
  }: {
    store: StoreInfo;
    stores: StoreInfo[];
    onchange: () => void;
    onswitch: (id: string) => void;
    onadd: () => void;
  } = $props();

  const SYNC_EVERY_MS = 60_000;
  const AUTOSAVE_MS = 700;
  /** How long after the last edit to sync, so a change reaches other devices without a button press. */
  const SYNC_AFTER_EDIT_MS = 5_000;
  const lockMinutes = () => {
    try {
      return Number(localStorage.getItem("flint.autoLockMinutes") ?? 15);
    } catch {
      return 15;
    }
  };

  let files = $state<string[]>([]);
  let expanded = $state(new Set<string>());
  let current = $state<{ path: string; text: string } | null>(null);
  let dirty = false;
  let saveState = $state<"saved" | "saving" | "unsaved" | "failed">("saved");
  let backlinks = $state<string[]>([]);
  let query = $state("");
  let hits = $state<Hit[] | null>(null);
  let status = $state("");
  let statusIsError = $state(false);
  let syncing = $state(false);
  let creating = $state(false);
  let newName = $state("");
  let conflictOpen = $state<StoreInfo["conflicts"][number] | null>(null);
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let syncTimer: ReturnType<typeof setTimeout> | undefined;
  /** Counts local changes, so a sync can tell whether more happened while it ran. */
  let changes = 0;
  let unsynced = false;
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  let lastActivity = Date.now();

  const tree = $derived(buildTree(files));
  const conflicted = $derived(new Set(store.conflicts.map((c) => c.path)));
  const lockedByConflicts = $derived(store.conflicts.length);

  function say(message: string, error = false) {
    status = message;
    statusIsError = error;
  }

  /** Something changed here that other devices have not seen: say so, and sync soon. */
  function markUnsynced() {
    changes++;
    unsynced = true;
    say("Changes not synced yet");
    clearTimeout(syncTimer);
    syncTimer = setTimeout(() => sync(), SYNC_AFTER_EDIT_MS);
  }

  async function loadFiles() {
    files = await api.files(store.id);
  }

  async function loadBacklinks(path: string) {
    try {
      backlinks = (await api.backlinks(store.id, path)).map((b) => b.from);
    } catch {
      backlinks = [];
    }
  }

  /** Writes the open note now, if it has unsaved changes. */
  async function flush(): Promise<boolean> {
    clearTimeout(saveTimer);
    if (!dirty || !current) return true;
    const { path, text } = current;
    saveState = "saving";
    try {
      await api.saveNote(store.id, path, text);
      if (current?.path === path && current.text === text) {
        dirty = false;
        saveState = "saved";
      }
      return true;
    } catch (e) {
      saveState = "failed";
      say(`Could not save ${path}: ${errorMessage(e)}`, true);
      return false;
    }
  }

  function edited(text: string) {
    if (!current) return;
    current.text = text;
    dirty = true;
    saveState = "unsaved";
    markUnsynced();
    clearTimeout(saveTimer);
    saveTimer = setTimeout(flush, AUTOSAVE_MS);
  }

  async function openNote(path: string) {
    if (current?.path === path) return;
    if (!(await flush())) return;
    try {
      const text = await api.readNote(store.id, path);
      current = { path, text };
      dirty = false;
      saveState = "saved";
      hits = null;
      query = "";
      revealFolders(path);
      loadBacklinks(path);
    } catch (e) {
      say(`Could not open ${path}: ${errorMessage(e)}`, true);
    }
  }

  function revealFolders(path: string) {
    const parts = path.split("/").slice(0, -1);
    const next = new Set(expanded);
    parts.forEach((_, i) => next.add(parts.slice(0, i + 1).join("/")));
    expanded = next;
  }

  function toggle(path: string) {
    const next = new Set(expanded);
    if (!next.delete(path)) next.add(path);
    expanded = next;
  }

  async function openLink(target: string) {
    if (!current) return;
    try {
      const path = await api.resolveLink(store.id, current.path, target);
      if (path) await openNote(path);
      else if (confirm(`"${target}" does not exist yet. Create it?`)) await createNote(target);
    } catch (e) {
      say(errorMessage(e), true);
    }
  }

  async function createNote(name: string) {
    const clean = name.trim().replace(/^\/+/, "");
    if (!clean) return;
    const path = isNote(clean) || clean.includes(".") ? clean : `${clean}.md`;
    if (!(await flush())) return;
    try {
      if (!files.includes(path)) {
        await api.saveNote(store.id, path, "");
        await loadFiles();
        markUnsynced();
      }
      await openNote(path);
    } catch (e) {
      say(`Could not create ${path}: ${errorMessage(e)}`, true);
    }
  }

  async function submitNew(event: Event) {
    event.preventDefault();
    const name = newName;
    creating = false;
    newName = "";
    await createNote(name);
  }

  function searchSoon() {
    clearTimeout(searchTimer);
    if (!query.trim()) {
      hits = null;
      return;
    }
    searchTimer = setTimeout(async () => {
      try {
        hits = await api.search(store.id, query);
      } catch (e) {
        say(errorMessage(e), true);
      }
    }, 150);
  }

  async function sync(manual = false) {
    if (syncing) return;
    syncing = true;
    clearTimeout(syncTimer);
    if (manual || unsynced) say("Syncing…");
    const changesAtStart = changes;
    try {
      if (!(await flush())) return;
      const result = await api.sync(store.id);
      await loadFiles();
      onchange();
      if (current && !dirty && files.includes(current.path)) {
        // another device may have changed the open note
        const fresh = await api.readNote(store.id, current.path).catch(() => null);
        if (fresh !== null && fresh !== current.text && !dirty) current = { path: current.path, text: fresh };
      } else if (current && !files.includes(current.path)) {
        current = null;
      }
      if (current) loadBacklinks(current.path);
      const problems = [...result.failed, ...result.refused];
      if (changes !== changesAtStart) {
        // edited while syncing: what was just synced is not what is on screen
        markUnsynced();
      } else if (problems.length) say(`Synced with problems: ${problems[0]}${problems.length > 1 ? ` (+${problems.length - 1} more)` : ""}`, true);
      else if (result.conflicts.length) say(result.conflicts[0] + " — see Conflicts", true);
      else say(`Synced: ${summarise(result)}`);
      if (changes === changesAtStart && problems.length === 0) unsynced = false;
    } catch (e) {
      if (errorKind(e) === "locked") onchange();
      say(`Sync failed: ${errorMessage(e)}${unsynced ? " (changes not synced yet)" : ""}`, true);
      if (unsynced) {
        // try again in a while, not in a tight loop
        clearTimeout(syncTimer);
        syncTimer = setTimeout(() => sync(), SYNC_EVERY_MS);
      }
    } finally {
      syncing = false;
    }
  }

  /** Leaves this store (to another, or to the add-store screen) without losing an edit still waiting to be saved. */
  async function leaveThen(next: () => void) {
    await flush();
    if (unsynced) await sync();
    next();
  }

  async function lock() {
    await flush();
    if (unsynced) await sync(); // locking stops syncing, so send what is waiting first
    try {
      await api.lock(store.id);
    } finally {
      current = null;
      onchange();
    }
  }

  async function conflictsChanged() {
    conflictOpen = null;
    await loadFiles();
    onchange();
    if (current) {
      const fresh = await api.readNote(store.id, current.path).catch(() => null);
      if (fresh !== null) current = { path: current.path, text: fresh };
    }
    sync();
  }

  function noteActivity() {
    lastActivity = Date.now();
  }

  let timers: ReturnType<typeof setInterval>[] = [];
  onMount(async () => {
    try {
      await loadFiles();
    } catch (e) {
      say(errorMessage(e), true);
    }
    timers = [
      setInterval(() => sync(), SYNC_EVERY_MS),
      setInterval(() => {
        const minutes = lockMinutes();
        if (minutes > 0 && Date.now() - lastActivity > minutes * 60_000) lock();
      }, 10_000),
    ];
    sync();
  });
  onDestroy(() => {
    timers.forEach(clearInterval);
    clearTimeout(saveTimer);
    clearTimeout(syncTimer);
    clearTimeout(searchTimer);
  });
</script>

<svelte:window onkeydown={noteActivity} onmousedown={noteActivity} onmousemove={noteActivity} />

<div class="app">
  <aside>
    <header>
      {#if stores.length > 1}
        <select value={store.id} onchange={(e) => {
            const id = e.currentTarget.value;
            leaveThen(() => onswitch(id));
          }} aria-label="Store">
          {#each stores as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
        </select>
      {:else}
        <strong class="storename">{store.name}</strong>
      {/if}
      <button title="New note" aria-label="New note" onclick={() => (creating = true)}>＋</button>
    </header>
    <input
      class="search"
      type="search"
      placeholder="Search"
      bind:value={query}
      oninput={searchSoon}
      aria-label="Search notes"
    />
    {#if creating}
      <form onsubmit={submitNew} class="new">
        <!-- svelte-ignore a11y_autofocus -->
        <input placeholder="Note name (folders: a/b)" bind:value={newName} autofocus onblur={() => !newName && (creating = false)} />
      </form>
    {/if}
    <nav>
      {#if hits}
        {#if hits.length === 0}
          <p class="dim pad">No matches.</p>
        {/if}
        {#each hits as hit (hit.path)}
          <button class="hit" onclick={() => openNote(hit.path)}>
            <span class="hit-title">{title(hit.path)}</span>
            <span class="hit-snippet dim">
              {#each pieces(hit.snippet) as piece}{#if piece.match}<mark>{piece.text}</mark>{:else}{piece.text}{/if}{/each}
            </span>
          </button>
        {/each}
      {:else}
        <FileTree nodes={tree} selected={current?.path ?? null} {expanded} {conflicted} ontoggle={toggle} onopen={openNote} />
        {#if files.length === 0}
          <p class="dim pad">
            No notes yet. Make one with ＋, or drop markdown files into<br /><code>{store.folder}</code>
          </p>
        {/if}
      {/if}
    </nav>
    <footer>
      <button onclick={() => sync(true)} disabled={syncing}>{syncing ? "Syncing…" : "Sync now"}</button>
      <button onclick={() => leaveThen(onadd)}>Add store</button>
      <button onclick={lock}>Lock</button>
    </footer>
  </aside>

  <main>
    {#if current}
      <div class="titlebar">
        <span class="title">{title(current.path)}</span>
        <span class="dim path">{current.path}</span>
        <span class="save dim" class:error={saveState === "failed"}>
          {saveState === "saved" ? "" : saveState === "saving" ? "Saving…" : saveState === "unsaved" ? "Editing…" : "Not saved"}
        </span>
      </div>
      {#key current.path}
        <div class="editor-host">
          <Editor text={current.text} onchange={edited} onopenlink={openLink} />
        </div>
      {/key}
    {:else}
      <div class="empty dim">
        <p>Open a note from the sidebar, or press ＋ to make one.</p>
        <p>In a note, click a <code>[[link]]</code> to follow it (arrow keys move into it to edit it).</p>
      </div>
    {/if}
    <div class="statusbar" class:error={statusIsError}>
      <span>{status || (syncing ? "Syncing…" : "")}</span>
    </div>
  </main>

  <section class="right">
    <h4>Backlinks</h4>
    {#if !current}
      <p class="dim pad">—</p>
    {:else if backlinks.length === 0}
      <p class="dim pad">Nothing links here.</p>
    {:else}
      {#each backlinks as from (from)}
        <button class="link pad" onclick={() => openNote(from)}>{title(from)}</button>
      {/each}
    {/if}

    {#if lockedByConflicts}
      <h4 class="error">Conflicts ({lockedByConflicts})</h4>
      {#each store.conflicts as c (c.path)}
        <button class="link pad" onclick={() => (conflictOpen = c)}>{c.path}</button>
      {/each}
    {/if}
    {#if store.awaitingEncryption.length}
      <h4>Waiting to be encrypted</h4>
      <p class="dim pad">{store.awaitingEncryption.length} dropped-in file(s); the next sync encrypts them.</p>
    {/if}
  </section>
</div>

{#if conflictOpen}
  <ConflictView storeId={store.id} conflict={conflictOpen} onclose={() => (conflictOpen = null)} onresolved={conflictsChanged} />
{/if}

<style>
  .app {
    display: grid;
    grid-template-columns: 260px 1fr 220px;
    height: 100%;
  }
  aside,
  .right {
    background: var(--bg-side);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  aside {
    border-right: 1px solid var(--border);
  }
  .right {
    border-left: 1px solid var(--border);
    overflow-y: auto;
  }
  aside header {
    display: flex;
    gap: 6px;
    padding: 10px 10px 6px;
    align-items: center;
  }
  .storename {
    flex: 1;
    padding-left: 4px;
  }
  aside header select {
    flex: 1;
  }
  .search,
  .new {
    margin: 0 10px 8px;
    width: auto;
  }
  .new input {
    margin: 0;
  }
  nav {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 6px;
  }
  aside footer {
    display: flex;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border);
  }
  aside footer button {
    flex: 1;
    padding-inline: 4px;
  }
  main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .titlebar {
    display: flex;
    gap: 12px;
    align-items: baseline;
    padding: 10px 18px 8px;
    border-bottom: 1px solid var(--border);
  }
  .title {
    font-weight: 600;
    font-size: 15px;
  }
  .path {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12px;
  }
  .save {
    font-size: 12px;
  }
  .editor-host {
    flex: 1;
    min-height: 0;
  }
  .empty {
    flex: 1;
    display: grid;
    align-content: center;
    justify-items: center;
    gap: 2px;
  }
  .statusbar {
    padding: 4px 18px;
    border-top: 1px solid var(--border);
    font-size: 12px;
    color: var(--fg-dim);
    min-height: 26px;
  }
  .statusbar.error {
    color: var(--danger);
  }
  h4 {
    margin: 14px 12px 4px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--fg-dim);
  }
  .pad {
    padding: 2px 12px;
    margin: 0;
  }
  button.link.pad {
    display: block;
    width: 100%;
    padding: 3px 12px;
  }
  .hit {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    background: none;
    border: none;
    text-align: left;
    padding: 6px 8px;
  }
  .hit-title {
    font-weight: 600;
  }
  .hit-snippet {
    font-size: 12px;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  code {
    word-break: break-all;
  }
</style>
