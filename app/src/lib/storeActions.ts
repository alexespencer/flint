import { api, errorKind, errorMessage } from "./api";

/**
 * Takes a store off this device, asking first. The store stays on S3 and can be joined again; its folder is
 * renamed, not deleted. The store is synced first, and if that cannot be done (offline, locked) or something
 * would be left only on this device, the reason is shown and the person can remove it anyway (the folder is
 * kept, so nothing is lost). Returns what happened, or `null` if they cancelled.
 */
export async function removeStoreFlow(
  store: { id: string; name: string },
  options: { force?: boolean } = {},
): Promise<{ keptAs: string | null } | null> {
  const ok = confirm(
    `Remove “${store.name}” from this device?\n\nIt stays on S3, and you can join it again at any time. ` +
      `Its folder here is renamed (${store.name}.removed-…), not deleted.`,
  );
  if (!ok) return null;
  let force = options.force ?? false;
  for (;;) {
    try {
      return { keptAs: await api.removeStore(store.id, force) };
    } catch (e) {
      if (force) throw e;
      const kind = errorKind(e);
      const why = kind === "locked" ? "The store is locked, so it cannot be synced first." : errorMessage(e);
      if (!confirm(`${why}\n\nRemove it anyway? The folder is kept, so nothing is lost, but anything not yet on S3 stays only in that folder.`)) {
        return null;
      }
      force = true;
    }
  }
}
