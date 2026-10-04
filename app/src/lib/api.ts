import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

export interface Conflict {
  path: string;
  /** Where the other version was written; null when it was deleted on the other side. */
  copy: string | null;
}

export interface StoreInfo {
  /** The S3 account, for reusing it. The secret key is never sent to the UI. */
  bucket: string;
  endpoint: string | null;
  region: string;
  accessKeyId: string;
  id: string;
  name: string;
  folder: string;
  active: boolean;
  unlocked: boolean;
  files: number;
  awaitingEncryption: string[];
  conflicts: Conflict[];
}

export interface SyncResult {
  adopted: number;
  uploaded: number;
  downloaded: number;
  deletedRemote: number;
  trashedLocal: number;
  conflicts: string[];
  refused: string[];
  failed: string[];
}

export interface Hit {
  path: string;
  snippet: string;
}

export interface NewStore {
  name: string;
  folder: string;
  bucket: string;
  prefix: string;
  region: string;
  endpoint: string | null;
  accessKeyId: string;
  /** Empty when `reuseSecretFrom` names a store whose saved secret key is used. */
  s3Secret: string;
  reuseSecretFrom: string | null;
  password: string;
}

/** An error from the Rust side: a stable `kind` to branch on and a message to show. */
export interface CommandError {
  kind: "locked" | "wrongPassword" | "storeExists" | "noSuchStore" | "passwordChangedElsewhere" | "other";
  message: string;
}

export function errorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) return String((e as CommandError).message);
  return String(e);
}

export function errorKind(e: unknown): CommandError["kind"] | undefined {
  return e && typeof e === "object" && "kind" in e ? (e as CommandError).kind : undefined;
}

export const api = {
  listStores: () => invoke<StoreInfo[]>("list_stores"),
  defaultFolder: (name: string) => invoke<string | null>("default_folder", { name }),
  createStore: (args: NewStore) => invoke<string>("create_store", { args }),
  joinStore: (args: NewStore) => invoke<string>("join_store", { args }),
  unlock: (id: string, password: string) => invoke<void>("unlock", { id, password }),
  lock: (id: string) => invoke<void>("lock", { id }),
  switchStore: (id: string) => invoke<void>("switch_store", { id }),
  forgetStore: (id: string) => invoke<void>("forget_store", { id }),
  changePassword: (id: string, old: string, next: string) =>
    invoke<void>("change_password", { id, old, new: next }),
  sync: (id: string) => invoke<SyncResult>("sync", { id }),
  resolve: (id: string, path: string) => invoke<void>("resolve", { id, path }),
  files: (id: string) => invoke<string[]>("files", { id }),
  readNote: (id: string, path: string) => invoke<string>("read_note", { id, path }),
  saveNote: (id: string, path: string, text: string) => invoke<void>("save_note", { id, path, text }),
  search: (id: string, query: string) => invoke<Hit[]>("search", { id, query }),
  backlinks: (id: string, path: string) => invoke<{ from: string }[]>("backlinks", { id, path }),
  resolveLink: (id: string, from: string, target: string) =>
    invoke<string | null>("resolve_link", { id, from, target }),
};

/** Asks the user for a folder. */
export async function pickFolder(): Promise<string | null> {
  const chosen = await open({ directory: true, multiple: false });
  return typeof chosen === "string" ? chosen : null;
}

export function summarise(r: SyncResult): string {
  const parts = [
    r.adopted && `${r.adopted} encrypted`,
    r.uploaded && `${r.uploaded} up`,
    r.downloaded && `${r.downloaded} down`,
    r.deletedRemote && `${r.deletedRemote} deleted remotely`,
    r.trashedLocal && `${r.trashedLocal} moved to trash`,
  ].filter(Boolean);
  return parts.length ? parts.join(", ") : "up to date";
}
