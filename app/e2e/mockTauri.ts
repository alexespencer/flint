import type { Page } from "@playwright/test";

/** What the fake Rust side holds: one unlocked store and its notes. */
export interface Fixture {
  storeName: string;
  /** Path -> markdown, e.g. `"Projects/Flint.md"`. */
  notes: Record<string, string>;
  /** Notes flagged as in conflict, with the path of the other version's copy. */
  conflicts?: { path: string; copy: string | null }[];
}

/**
 * Stands in for Tauri's IPC, so the frontend runs in a plain browser. Every `invoke` the app makes
 * (app/src/lib/api.ts) is answered from the fixture; a command it does not know is logged and answers null.
 */
export async function mockTauri(page: Page, fixture: Fixture) {
  await page.addInitScript((fx: Fixture) => {
    const store = {
      bucket: "notes",
      endpoint: null,
      region: "auto",
      accessKeyId: "AKIAEXAMPLE",
      id: "store-1",
      name: fx.storeName,
      folder: `~/Flint/${fx.storeName}`,
      active: true,
      unlocked: true,
      passwordless: false,
      files: Object.keys(fx.notes).length,
      awaitingEncryption: [],
      conflicts: fx.conflicts ?? [],
    };
    const noSync = { adopted: 0, uploaded: 0, downloaded: 0, deletedRemote: 0, trashedLocal: 0, conflicts: [], refused: [], failed: [], settingsChanged: false, settingsError: null };
    const stem = (path: string) => path.replace(/^.*\//, "").replace(/\.md$/i, "");
    const links = (text: string) => [...text.matchAll(/\[\[([^\]|#]+)/g)].map((m) => m[1].trim());

    const handlers: Record<string, (args: any) => unknown> = {
      list_stores: () => [store],
      switch_store: () => null,
      resolve: () => null,
      files: () => Object.keys(fx.notes),
      folders: () => [],
      read_note: ({ path }) => fx.notes[path] ?? "",
      save_note: ({ path, text }) => void (fx.notes[path] = text),
      get_settings: () => ({ attachmentFolder: "Images", newNoteFolder: "", newerVersion: null }),
      sync: () => noSync,
      search: ({ query }) =>
        Object.entries(fx.notes)
          .filter(([path, text]) => `${path}\n${text}`.toLowerCase().includes(String(query).toLowerCase()))
          .map(([path, text]) => ({ path, snippet: text.slice(0, 80) })),
      backlinks: ({ path }) =>
        Object.entries(fx.notes)
          .filter(([from, text]) => from !== path && links(text).includes(stem(path)))
          .map(([from]) => ({ from })),
      resolve_link: ({ target }) => Object.keys(fx.notes).find((path) => stem(path) === target) ?? null,
      biometric_state: () => ({ available: false, enabled: false }),
      "plugin:window|set_theme": () => null,
      "plugin:app|version": () => "0.0.0",
    };

    const internals = {
      metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
      transformCallback: () => 0,
      unregisterCallback: () => {},
      convertFileSrc: (path: string) => path,
      invoke: async (command: string, args: unknown) => {
        const handler = handlers[command];
        if (!handler) {
          console.warn(`mockTauri: no handler for ${command}`);
          return null;
        }
        return handler(args ?? {});
      },
    };
    (window as any).__TAURI_INTERNALS__ = internals;
  }, fixture);
}
