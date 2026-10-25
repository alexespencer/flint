# Roadmap

What is built, and what is left. Flint is one person's project, so this is a list of intentions, not promises, and the order is roughly the order I would do them in.

## Next

**Windows**

- Skip and warn about notes whose names Windows cannot hold (`:`, `?`, a trailing dot, `CON`, ...), instead of failing the file.
- Windows Hello, as the fingerprint equivalent.

**Desktop and Android apps**

- A file watcher, so files dropped into a store folder are encrypted at once (today: at the next sync).
- A warning when the store folder holds hidden folders such as `.git`, which stay readable and are not synced.
- A screen for changing a store's password (the command line can already do it), including setting one on a store that was made without.
- A real progress indicator for sync (the engine knows how many files it will move before it starts).
- Restore from trash, and drag and drop to move files and folders.
- Live-preview rendering of emphasis and links in the editor (headings, inline code and code blocks are already rendered), and a highlighted diff in the conflict view.
- A visual refresh: spacing, typography, the sidebar and header.

**Sync and robustness**

- Run sync off the worker thread that serves note reads, so a long first sync does not pause opening notes.
- A "restore from S3" action for a local file that no longer decrypts (sync already refuses to upload it).
- A process-crash step in the sync simulation, and a measured first sync of a large vault.
- Zero keys in memory on lock (today they are removed from the keychain but not wiped from memory).
- The skip-rehash hint can miss an edit that keeps the same size and modification time.

**Releases**

- A smaller release profile (strip symbols, link-time optimisation; the APK is about 33 MB today).
- A `.dmg` for Intel Macs (today's is Apple Silicon only). The Windows and macOS installers are not signed.
- Self-updating Mac and Windows apps (Tauri's updater plugin; Android updates through Obtainium already, see [`releasing.md`](releasing.md)).

## Later

- Rename with `[[link]]` rewriting (renaming a note today does not update links that point to it).
- "Open in external editor": decrypt a note to a temporary file, watch it, and re-encrypt on save, so any editor works without leaving plaintext in the store folder (today notes are binary files and only plaintext replacement works).
- Android background sync (WorkManager).
- Automatic resolution of simple conflicts: a three-way merge, as git does, when the two devices changed different parts of a note (the last synced version is the common ancestor). Edits to the same lines would still be kept as two versions, and a merge would be reported and undoable. Today every conflict is kept for you to resolve.
- Full key rotation (re-encrypting every file under a new data key).
- Moving several notes at once.

## Known small things

- In a numbered checklist, Enter continues the numbering without the checkbox (a quirk of the markdown library).
- A folder with no file in it exists on one device only: S3 has no folders, only file paths.
