# The `flint` CLI

A headless shell over `flint-app-service`, the same API the app will use.

```sh
flint create-store Notes --bucket flint --prefix notes --endpoint https://<account>.r2.cloudflarestorage.com --access-key-id <id>
flint join Notes --bucket flint --prefix notes --endpoint https://... --access-key-id <id>   # on a second device
flint sync | status | resolve <path> | passwd | lock | unlock | stores | switch <name> | adopt | remove <name> | delete-store <name> | reupload
```

- Prompts for the S3 secret key and passwords without echo. `--password-stdin` reads them from standard input, one per
  line, in the order they would be asked (create: secret key, password, repeat; join: secret key, password).
- The list of stores is `stores.json` in the config folder (`~/Library/Application Support/Flint` on macOS,
  or `--config-dir` / `FLINT_CONFIG_DIR`). The S3 secret key and an unlocked store's data key are in the login
  keychain (service `dev.flint`), never in that file. The password is never kept.
- Each store's `sync.db` and `index.db` live in `<config>/stores/<id>/`, not in the store folder.
- The store folder defaults to `~/Flint/<name>`; the prefix defaults to the name.
- Only macOS has a secret store so far; elsewhere nothing is remembered between runs.
- `cargo test -p flint-registry -- --ignored` runs the keychain round trip against the real login keychain.
- `FLINT_SAMPLE_VAULT=<dir> cargo test -p flint-app-service -- --ignored` round-trips a copy of a real vault between
  two devices (in memory, no network).

## Removing and deleting a store

- `flint remove <name>` takes a store off this device. It stays on S3 (join it again any time) and its folder is renamed
  to `<name>.removed-<date>-<time>`, not deleted. The store is synced first, and the removal is refused if anything would
  be left only here (a failed or refused file, a conflict, a locked store); `--force` skips that check. (`forget` is an
  alias.)
- `flint delete-store <name>` deletes every file of the store from S3, for every device, then removes it here. You type
  the store's name to confirm. It cannot be undone.
- If a store vanishes from S3 (deleted there, or the bucket emptied), `flint sync` stops and changes nothing on the
  device, instead of reading it as "every file was deleted". `flint reupload` puts this device's copy back.
