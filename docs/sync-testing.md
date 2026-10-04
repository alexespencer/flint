# Testing sync

Sync is the part of Flint where a bug loses someone's writing, so it has three layers of tests in `flint-sync`
(all on in-memory stores; real S3 behaviour is covered by `flint-remote`'s own contract suite).

1. **The planner** (`plan/tests.rs`): a pure function, one test per row of the plan's rules table, plus a property
   test that states *per action* that nothing is overwritten or removed unless it is unchanged since the last
   agreement or already on the other side.
2. **The executor** (`engine/tests.rs`): two devices sharing one remote, scripted failures (lost reply, refused
   retry, rival write mid-sync, flaky network, a local edit made while sync is running), and the guards that keep
   plaintext off the remote and non-ciphertext off the disk.
3. **The simulation** (`simulation.rs`): three devices, random writes, deletes, renames, identical "twin" edits,
   conflict resolutions, syncs in any order, and injected network faults. After everything is settled it checks:
   - **no write is lost**: every saved version records the version its author was looking at, forming a graph; any
     version nothing was later built on must still exist somewhere (a deletion only supersedes what it deleted);
   - **everything stored is ciphertext**, on the remote and on every device;
   - **devices converge**: every path not held in an unresolved conflict is identical everywhere.

```sh
cargo test -p flint-sync                               # 300 simulated runs, about 2 s
PROPTEST_CASES=10000 cargo test -p flint-sync simulation   # the M3 gate: about 70 s
```

Proptest shrinks a failure to a minimal sequence of steps and saves its seed in `crates/flint-sync/proptest-regressions/`
(kept in git, so a fixed bug stays fixed).

## What the simulation found

- **`resolve` overwrote an edit nobody had seen** (found on the first run, shrunk to 8 steps). After a conflict the
  user's merge was made from the conflict copy, but `resolve` adopted the remote's *current* version as the baseline.
  If the remote had moved on in between, the merge overwrote that newer edit. Fixed: the conflict records the remote
  version the copy was made from, and resolving uses that. If the remote moved on, both versions are kept again.
  Regression tests: `resolving_after_the_remote_moved_on_does_not_overwrite_the_edit_nobody_saw` and the saved seed.

The suite was checked by breaking the code on purpose (re-introducing that bug; treating differing contents as
equal; deleting a remote file that had changed; and several executor guards): each mutation is caught.
