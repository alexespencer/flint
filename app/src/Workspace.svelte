<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { api, errorKind, errorMessage, isAndroid, summarise, type FolderInfo, type Hit, type StoreInfo } from "./lib/api";
  import { backLayer } from "./lib/layers";
  import { autoLockMinutes } from "./lib/settings";
  import { pieces } from "./lib/snippet";
  import { baseName, buildTree, isNote, isWithin, moveTargets, parentOf, title, type TreeNode } from "./lib/tree";
  import ConflictView from "./ConflictView.svelte";
  import ContextMenu, { type MenuItem } from "./ContextMenu.svelte";
  import FolderPicker from "./FolderPicker.svelte";
  import PathDialog from "./PathDialog.svelte";
  import Editor from "./Editor.svelte";
  import FileTree from "./FileTree.svelte";
  import Settings from "./Settings.svelte";

  let {
    store,
    stores,
    onchange,
    onswitch,
    onadd,
    onlocked,
  }: {
    store: StoreInfo;
    stores: StoreInfo[];
    onchange: () => void;
    onswitch: (id: string) => void;
    onadd: () => void;
    /** The app was locked. `askNext`: the unlock screen should ask for the fingerprint by itself. */
    onlocked: (askNext: boolean) => void;
  } = $props();

  const SYNC_EVERY_MS = 60_000;
  const AUTOSAVE_MS = 700;
  /** How long after the last edit to sync, so a change reaches other devices without a button press. */
  const SYNC_AFTER_EDIT_MS = 5_000;

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
  let folderInfos = $state<FolderInfo[]>([]);
  const folders = $derived(folderInfos.map((f) => f.path));
  let creating = $state<null | "note" | "folder">(null);
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  let renaming = $state<{ path: string; folder: boolean; links: number } | null>(null);
  let picking = $state<{ path: string; targets: string[] } | null>(null);
  let showSettings = $state(false);
  let newName = $state("");
  let conflictOpen = $state<StoreInfo["conflicts"][number] | null>(null);
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let syncTimer: ReturnType<typeof setTimeout> | undefined;
  /** Counts local changes, so a sync can tell whether more happened while it ran. */
  let changes = 0;
  let unsynced = false;
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  let lastActivity = Date.now();
  /** A narrow screen shows one pane at a time: the file list, or the open note. */
  let narrow = $state(false);
  let pane = $state<"list" | "note">("list");
  let showInfo = $state(false);

  const tree = $derived(buildTree(files, folders));
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
    // The folder list only adds empty folders and creation times: if it cannot be had (or is not in the shape
    // expected, as in a half-updated dev build), the notes must still show.
    folderInfos = await api.folders(store.id).then(
      (list) => (Array.isArray(list) ? list.filter((f) => typeof f?.path === "string") : []),
      () => [],
    );
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
      pane = "note";
      showInfo = false;
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

  async function createFolder(name: string) {
    const clean = name.trim().replace(/^\/+|\/+$/g, "");
    if (!clean) return;
    try {
      await api.createFolder(store.id, clean);
      await loadFiles();
      revealFolders(`${clean}/x`); // opens the new folder and the ones above it
    } catch (e) {
      say(`Could not create the folder ${clean}: ${errorMessage(e)}`, true);
    }
  }

  async function submitNew(event: Event) {
    event.preventDefault();
    const name = newName;
    const kind = creating;
    creating = null;
    newName = "";
    if (kind === "folder") await createFolder(name);
    else await createNote(name);
  }

  /** Starts typing the name of a new note or folder, inside `parent` when given. */
  function startCreating(kind: "note" | "folder", parent = "") {
    creating = kind;
    newName = parent ? `${parent}/` : "";
    if (parent) expanded = new Set(expanded).add(parent);
  }

  // ---- the actions on a file or folder ----------------------------------------------------------

  function showMenu(node: TreeNode, x: number, y: number) {
    const folder = !!node.children;
    const items: MenuItem[] = [];
    if (folder) {
      items.push({ label: "New note here", action: () => startCreating("note", node.path) });
      items.push({ label: "New folder here", action: () => startCreating("folder", node.path) });
    }
    items.push({ label: "Rename…", action: () => askRename(node) });
    items.push({ label: "Move to…", action: () => showMoveMenu(node, x, y) });
    items.push({ label: "Move to the trash…", danger: true, action: () => deletePath(node) });
    menu = { x, y, items };
  }

  function showNewMenu(event: MouseEvent) {
    const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
    menu = {
      x: box.left,
      y: box.bottom + 4,
      items: [
        { label: "New note", action: () => startCreating("note") },
        { label: "New folder", action: () => startCreating("folder") },
      ],
    };
  }

  /** The most recently created folders to move to, and a way to pick from all of them when there are more. */
  const MOVE_SHORTLIST = 5;

  function showMoveMenu(node: TreeNode, x: number, y: number) {
    const targets = moveTargets(node.path, folderInfos);
    const items: MenuItem[] = targets.slice(0, MOVE_SHORTLIST).map((target) => ({
      label: target || "Top level",
      action: () => moveInto(node.path, target),
    }));
    if (targets.length > MOVE_SHORTLIST) {
      items.push({ label: `More folders… (${targets.length - MOVE_SHORTLIST})`, action: () => (picking = { path: node.path, targets }) });
    }
    if (items.length === 0) items.push({ label: "No other folder: make one with ＋", disabled: true, action: () => {} });
    menu = { x, y, items };
  }

  function moveInto(path: string, folder: string) {
    return doMove(path, folder ? `${folder}/${baseName(path)}` : baseName(path));
  }

  async function askRename(node: TreeNode) {
    const folder = !!node.children;
    // links name a note, so renaming one is what can break them
    const links = folder ? 0 : await api.backlinks(store.id, node.path).then((b) => b.length, () => 0);
    renaming = { path: node.path, folder, links };
  }

  function nameProblem(value: string): string | null {
    return value.includes("/") ? "A name cannot contain “/”. Use Move to… to put it in another folder." : null;
  }

  function renameWarning(value: string): string | null {
    if (!renaming || renaming.folder || renaming.links === 0) return null;
    if (title(value) === title(renaming.path)) return null;
    const n = renaming.links;
    return `${n} note${n === 1 ? " links" : "s link"} to this one by its name. Links are not updated when a note is renamed, so ${n === 1 ? "that link" : "those links"} will stop working.`;
  }

  async function renameTo(name: string) {
    const target = renaming;
    renaming = null;
    if (!target) return;
    // a note renamed without typing an extension stays a .md
    const file = !target.folder && isNote(target.path) && !name.includes(".") ? `${name}.md` : name;
    const parent = parentOf(target.path);
    await doMove(target.path, parent ? `${parent}/${file}` : file);
  }

  async function doMove(from: string, to: string) {
    if (!(await flush())) return;
    try {
      const reopen = current && isWithin(current.path, from) ? to + current.path.slice(from.length) : null;
      const count = await api.movePath(store.id, from, to);
      if (reopen) {
        current = null;
        dirty = false;
      }
      await loadFiles();
      revealFolders(`${to}/x`);
      markUnsynced();
      if (reopen) await openNote(reopen);
      say(`Moved ${count} file${count === 1 ? "" : "s"}. Changes not synced yet`);
    } catch (e) {
      say(`Could not move ${from}: ${errorMessage(e)}`, true);
    }
  }

  async function deletePath(node: TreeNode) {
    const inside = node.children ? files.filter((f) => isWithin(f, node.path)).length : 1;
    const what = node.children ? `the folder "${node.name}"${inside ? ` and its ${inside} file${inside === 1 ? "" : "s"}` : ""}` : `"${title(node.path)}"`;
    if (!confirm(`Move ${what} to the trash?\n\nIt is removed from your other devices too, but kept in the .trash folder on each one.`)) return;
    if (!(await flush())) return;
    try {
      const count = await api.deletePath(store.id, node.path);
      if (current && isWithin(current.path, node.path)) {
        current = null;
        dirty = false;
        pane = "list";
      }
      await loadFiles();
      markUnsynced();
      say(`Moved ${count} file${count === 1 ? "" : "s"} to the trash. Changes not synced yet`);
    } catch (e) {
      say(`Could not delete ${node.path}: ${errorMessage(e)}`, true);
    }
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
        pane = "list";
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

  /** Locks every store. `askNext`: the person is coming back to the app (Android: from the background), so the
   *  unlock screen may ask for the fingerprint by itself; after the Lock button or an idle lock it does not. */
  async function lock(askNext = false) {
    await flush();
    // Locking stops syncing, so send what is waiting first. Not when the app is going to the background:
    // the network may already be gone, and a sync that hangs holds up unlocking when the person returns.
    if (unsynced && !askNext) await sync();
    try {
      await api.lockAll(); // "lock" means the app, not one store
    } finally {
      current = null;
      pane = "list";
      onlocked(askNext);
      onchange();
    }
  }

  /** On a phone, leaving the app locks the store, so nothing stays unlocked in the background. */
  function visibilityChanged() {
    if (isAndroid && document.visibilityState === "hidden") lock(true);
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

  // On a phone Back leaves the open note for the list (and closes the links sheet first), not the app.
  $effect(() => {
    if (narrow && pane === "note") {
      return backLayer(() => {
        flush();
        pane = "list";
      });
    }
  });
  $effect(() => {
    if (narrow && showInfo) return backLayer(() => (showInfo = false));
  });

  let timers: ReturnType<typeof setInterval>[] = [];
  onMount(async () => {
    const narrowScreen = window.matchMedia("(max-width: 720px)");
    narrow = narrowScreen.matches;
    narrowScreen.addEventListener("change", (e) => (narrow = e.matches));
    try {
      await loadFiles();
    } catch (e) {
      say(errorMessage(e), true);
    }
    timers = [
      setInterval(() => sync(), SYNC_EVERY_MS),
      setInterval(() => {
        const minutes = autoLockMinutes();
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
<svelte:document onvisibilitychange={visibilityChanged} />

<div class="app" class:narrow>
  <aside class:away={narrow && pane !== "list"}>
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
      <button title="New note or folder" aria-label="New note or folder" onclick={showNewMenu}>＋</button>
      <button class="icon" title="Settings" aria-label="Settings" onclick={() => (showSettings = true)}>⚙</button>
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
        <input
          placeholder={creating === "folder" ? "Folder name (nested: a/b)" : "Note name (in a folder: a/b)"}
          bind:value={newName}
          autofocus
          onblur={() => !newName && (creating = null)}
          onkeydown={(e) => e.key === "Escape" && ((creating = null), (newName = ""))}
        />
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
        <FileTree nodes={tree} selected={current?.path ?? null} {expanded} {conflicted} ontoggle={toggle} onopen={openNote} onmenu={showMenu} />
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
      <button onclick={() => lock()} title="Lock every store; each needs its password{isAndroid ? " or fingerprint" : ""} to open again">
        {stores.length > 1 ? "Lock all" : "Lock"}
      </button>
    </footer>
  </aside>

  <main class:away={narrow && pane !== "note"}>
    {#if current}
      <div class="titlebar">
        {#if narrow}
          <button class="back" aria-label="Back to notes" onclick={() => flush().then(() => (pane = "list"))}>‹</button>
        {/if}
        <span class="title">{title(current.path)}</span>
        <span class="dim path">{current.path}</span>
        <span class="save dim" class:error={saveState === "failed"}>
          {saveState === "saved" ? "" : saveState === "saving" ? "Saving…" : saveState === "unsaved" ? "Editing…" : "Not saved"}
        </span>
        {#if narrow}
          <button class="info" onclick={() => (showInfo = !showInfo)} aria-expanded={showInfo}>
            Links{backlinks.length ? ` (${backlinks.length})` : ""}
          </button>
        {/if}
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
    <div class="statusbar" class:error={statusIsError && !syncing}>
      {#if syncing}<div class="progress" role="progressbar" aria-label="Syncing"></div>{/if}
      <span>{syncing ? "Syncing…" : status}</span>
    </div>
  </main>

  <section class="right" class:sheet={narrow} class:open={showInfo}>
    {#if narrow}<button class="close" onclick={() => (showInfo = false)}>Close</button>{/if}
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

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />
{/if}

{#if renaming}
  <PathDialog
    title={renaming.folder ? "Rename folder" : "Rename"}
    help="Only the name changes; it stays in the same folder."
    initial={baseName(renaming.path)}
    confirmLabel="Rename"
    reject={nameProblem}
    warn={renameWarning}
    onconfirm={renameTo}
    onclose={() => (renaming = null)}
  />
{/if}

{#if picking}
  <FolderPicker
    title={`Move “${baseName(picking.path)}” to…`}
    targets={picking.targets}
    onpick={(folder) => {
      const path = picking!.path;
      picking = null;
      moveInto(path, folder);
    }}
    onclose={() => (picking = null)}
  />
{/if}

{#if showSettings}
  <Settings {store} {stores} onclose={() => (showSettings = false)} onchange={onchange} />
{/if}

{#if conflictOpen}
  <ConflictView storeId={store.id} conflict={conflictOpen} onclose={() => (conflictOpen = null)} onresolved={conflictsChanged} />
{/if}

<style>
  .app {
    display: grid;
    grid-template-columns: 260px 1fr 220px;
    height: 100%;
  }
  .app.narrow {
    grid-template-columns: 1fr;
  }
  .away {
    display: none !important;
  }
  .back,
  .info,
  .close {
    padding: 4px 10px;
  }
  .back {
    font-size: 20px;
    line-height: 1;
    align-self: center;
  }
  .right.sheet {
    display: none;
    position: fixed;
    inset: auto 0 0 0;
    max-height: 60%;
    border: 1px solid var(--border);
    border-radius: 14px 14px 0 0;
    box-shadow: 0 -6px 24px rgb(0 0 0 / 0.25);
    z-index: 5;
    padding-bottom: env(safe-area-inset-bottom);
  }
  .right.sheet.open {
    display: flex;
  }
  .close {
    align-self: flex-end;
    margin: 8px 10px 0;
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
  /* the gear glyph draws small at the normal size */
  button.icon {
    font-size: 20px;
    line-height: 1;
    padding: 4px 9px;
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
  .progress {
    position: absolute;
    inset: 0 0 auto 0;
    height: 2px;
    overflow: hidden;
    background: var(--border);
  }
  .progress::after {
    content: "";
    position: absolute;
    inset: 0 auto 0 0;
    width: 35%;
    background: var(--accent);
    animation: slide 1.1s ease-in-out infinite;
  }
  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(290%);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .progress::after {
      animation: none;
      width: 100%;
      opacity: 0.5;
    }
  }
  .statusbar {
    position: relative;
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
