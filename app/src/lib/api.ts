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
  /** The store's password is empty: nothing to ask for. (Still encrypted, but with a key anyone can derive.) */
  passwordless: boolean;
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

export interface BiometricState {
  /** This device can unlock with a fingerprint (hardware present, a finger enrolled). */
  available: boolean;
  /** This store has fingerprint unlock set up. */
  enabled: boolean;
}

export interface FolderInfo {
  path: string;
  /** When the folder was created on this device, in milliseconds since 1970. */
  createdMs: number;
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
  kind: "locked" | "inConflict" | "storeMissingOnRemote" | "notFullySynced" | "nameMismatch" | "alreadyExists" | "biometricCancelled" | "keyInvalidated" | "wrongPassword" | "storeExists" | "noSuchStore" | "passwordChangedElsewhere" | "other";
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
  lockAll: () => invoke<void>("lock_all"),
  switchStore: (id: string) => invoke<void>("switch_store", { id }),
  /** Takes the store off this device (its folder is renamed, not deleted); returns where the folder went. */
  removeStore: (id: string, force: boolean) => invoke<string | null>("remove_store", { id, force }),
  /** Deletes the store from S3 for every device (the name must be typed exactly), then removes it here. */
  deleteStore: (id: string, name: string) => invoke<number>("delete_store", { id, name }),
  /** Puts this device's copy back on S3 after the store was deleted there. */
  reuploadStore: (id: string) => invoke<SyncResult>("reupload_store", { id }),
  changePassword: (id: string, old: string, next: string) =>
    invoke<void>("change_password", { id, old, new: next }),
  sync: (id: string) => invoke<SyncResult>("sync", { id }),
  resolve: (id: string, path: string) => invoke<void>("resolve", { id, path }),
  files: (id: string) => invoke<string[]>("files", { id }),
  folders: (id: string) => invoke<FolderInfo[]>("folders", { id }),
  createFolder: (id: string, path: string) => invoke<void>("create_folder", { id, path }),
  deletePath: (id: string, path: string) => invoke<number>("delete_path", { id, path }),
  movePath: (id: string, from: string, to: string) => invoke<number>("move_path", { id, from, to }),
  /** The decrypted bytes of a file (an image, say). */
  readFile: (id: string, path: string) => invoke<ArrayBuffer>("read_file", { id, path }),
  /** Saves bytes into the store (a pasted picture); returns the path it got, which differs if the name was taken. */
  saveFile: (id: string, path: string, bytes: ArrayBuffer) =>
    invoke<string>("save_file", bytes, { headers: { "x-store": id, "x-path": encodeURIComponent(path) } }),
  readNote: (id: string, path: string) => invoke<string>("read_note", { id, path }),
  saveNote: (id: string, path: string, text: string) => invoke<void>("save_note", { id, path, text }),
  search: (id: string, query: string) => invoke<Hit[]>("search", { id, query }),
  backlinks: (id: string, path: string) => invoke<{ from: string }[]>("backlinks", { id, path }),
  resolveLink: (id: string, from: string, target: string) =>
    invoke<string | null>("resolve_link", { id, from, target }),
  biometricState: (id: string) => invoke<BiometricState>("biometric_state", { id }),
  biometricEnable: (id: string) => invoke<void>("biometric_enable", { id }),
  biometricUnlock: (id: string) => invoke<void>("biometric_unlock", { id }),
  biometricDisable: (id: string) => invoke<void>("biometric_disable", { id }),
};

/** Running in the Android app: no folder picker, and the app locks itself when it goes to the background. */
export const isAndroid = /Android/i.test(navigator.userAgent);

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
