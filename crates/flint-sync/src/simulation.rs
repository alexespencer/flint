//! The M3 simulation: three devices share one remote and a user does random things on them, in any
//! order, with the network misbehaving now and then. Afterwards everything is synced until nothing
//! changes, and two invariants are checked.
//!
//! **No write is lost.** Every version a user saves records the version they were looking at when they
//! saved it (its parent), so versions form a graph, and a deletion is a version too. A version that was
//! overwritten or deleted on purpose may disappear. But every version that nothing was later built on
//! (a leaf of the graph) must still exist somewhere at the end: in a device's folder, on the remote, or
//! as a conflict copy. Two devices editing the same file concurrently produce two leaves, so both must
//! survive.
//!
//! **Devices converge.** Once settled, every path that is not held in an unresolved conflict has the
//! same contents on every device and on the remote.
//!
//! Every written text is a unique token (`T17`), so a version can be found wherever it ended up.

use std::collections::{BTreeMap, BTreeSet};

use flint_remote::RemoteStore;
use flint_store_fs::LocalFs;
use proptest::prelude::*;

use crate::testing::{Fault, FaultyDevice, Op as RemoteOp, World, p};

const PATHS: [&str; 4] = ["a.md", "b.md", "dir/c.md", "dir/sub/d.md"];
const DEVICES: usize = 3;

#[derive(Debug, Clone)]
enum Step {
    Write {
        device: usize,
        path: usize,
    },
    Delete {
        device: usize,
        path: usize,
    },
    Rename {
        device: usize,
        from: usize,
        to: usize,
    },
    /// The same text typed on two devices at once (identical edits are not a conflict).
    Twin {
        devices: (usize, usize),
        path: usize,
    },
    /// The user settles a held conflict on this device: merges both versions, or keeps their own.
    Resolve {
        device: usize,
    },
    Sync {
        device: usize,
    },
    SyncAll,
    Flake {
        device: usize,
        fault: Flake,
    },
}

#[derive(Debug, Clone, Copy)]
enum Flake {
    FailUpload,
    LoseUploadReply,
    UploadAppliedThenRefused,
    FailDownload,
    FailListing,
    FailDelete,
}

fn step() -> impl Strategy<Value = Step> {
    let device = 0..DEVICES;
    let path = 0..PATHS.len();
    prop_oneof![
        30 => (device.clone(), path.clone()).prop_map(|(device, path)| Step::Write { device, path }),
        8 => (device.clone(), path.clone()).prop_map(|(device, path)| Step::Delete { device, path }),
        6 => (device.clone(), path.clone(), path.clone()).prop_map(|(device, from, to)| Step::Rename { device, from, to }),
        4 => (device.clone(), device.clone(), path.clone()).prop_map(|(a, b, path)| Step::Twin { devices: (a, b), path }),
        4 => device.clone().prop_map(|device| Step::Resolve { device }),
        25 => device.clone().prop_map(|device| Step::Sync { device }),
        5 => Just(Step::SyncAll),
        9 => (device, prop_oneof![
            Just(Flake::FailUpload),
            Just(Flake::LoseUploadReply),
            Just(Flake::UploadAppliedThenRefused),
            Just(Flake::FailDownload),
            Just(Flake::FailListing),
            Just(Flake::FailDelete),
        ])
        .prop_map(|(device, fault)| Step::Flake { device, fault }),
    ]
}

/// The graph of every version ever saved.
#[derive(Default)]
struct Versions {
    next: u32,
    parents: BTreeMap<String, BTreeSet<String>>,
    tombstones: BTreeSet<String>,
}

impl Versions {
    fn mint(&mut self, prefix: &str, parents: impl IntoIterator<Item = String>) -> String {
        self.next += 1;
        let token = format!("{prefix}{}", self.next);
        self.parents
            .insert(token.clone(), parents.into_iter().collect());
        token
    }

    /// A token that more than one device writes: the parents add up.
    fn extend(&mut self, token: &str, parents: impl IntoIterator<Item = String>) {
        self.parents
            .entry(token.to_owned())
            .or_default()
            .extend(parents);
    }

    /// Saved texts that nothing was built on afterwards: these must survive.
    fn leaves(&self) -> BTreeSet<String> {
        let has_children: BTreeSet<&String> = self.parents.values().flatten().collect();
        self.parents
            .keys()
            .filter(|t| !self.tombstones.contains(*t) && !has_children.contains(t))
            .cloned()
            .collect()
    }
}

struct Sim {
    world: World,
    devices: Vec<FaultyDevice>,
    versions: Versions,
}

impl Sim {
    fn new() -> Self {
        let world = World::new();
        let devices = (0..DEVICES)
            .map(|i| world.faulty_device(&format!("Device{i}")))
            .collect();
        Self {
            world,
            devices,
            versions: Versions::default(),
        }
    }

    /// What the user sees at this path on this device right now.
    fn current(&self, device: usize, path: &str) -> Option<String> {
        self.devices[device].read(path)
    }

    async fn apply(&mut self, step: &Step) {
        match *step {
            Step::Write { device, path } => {
                let path = PATHS[path];
                let parents: Vec<_> = self.current(device, path).into_iter().collect();
                let token = self.versions.mint("T", parents);
                self.devices[device].write(path, &token);
            }
            Step::Delete { device, path } => {
                let path = PATHS[path];
                if let Some(current) = self.current(device, path) {
                    let tombstone = self.versions.mint("D", [current]);
                    self.versions.tombstones.insert(tombstone);
                    self.devices[device].delete(path);
                }
            }
            Step::Rename { device, from, to } => {
                let (from, to) = (PATHS[from], PATHS[to]);
                if from != to
                    && self.current(device, to).is_none()
                    && let Some(current) = self.current(device, from)
                {
                    let moved = self.versions.mint("T", [current.clone()]);
                    let tombstone = self.versions.mint("D", [current]);
                    self.versions.tombstones.insert(tombstone);
                    // a rename: the old name goes, the new name holds the (re-encrypted) contents
                    self.devices[device].enc.rename(&p(from), &p(to)).unwrap();
                    self.devices[device].write(to, &moved);
                }
            }
            Step::Twin {
                devices: (a, b),
                path,
            } => {
                if a != b {
                    let path = PATHS[path];
                    let token = self.versions.mint("T", []);
                    for device in [a, b] {
                        let parents: Vec<_> = self.current(device, path).into_iter().collect();
                        self.versions.extend(&token, parents);
                        self.devices[device].write(path, &token);
                    }
                }
            }
            Step::Resolve { device } => {
                let Some(conflict) = self.devices[device]
                    .syncer
                    .state()
                    .conflicts()
                    .unwrap()
                    .into_iter()
                    .next()
                else {
                    return;
                };
                let path = conflict.path.as_str().to_owned();
                let mut parents: Vec<_> = self.current(device, &path).into_iter().collect();
                // The user read both versions and wrote a merge of them. (When the other side deleted the
                // file there is no copy, and they simply keep what they have.)
                if let Some(copy) = &conflict.copy {
                    parents.extend(self.current(device, copy.as_str()));
                    let merged = self.versions.mint("T", parents);
                    self.devices[device].write(&path, &merged);
                }
                self.devices[device]
                    .syncer
                    .resolve(&conflict.path)
                    .await
                    .unwrap();
            }
            Step::Sync { device } => {
                let _ = self.devices[device].syncer.sync().await; // an Err is a flaky network: carry on
            }
            Step::SyncAll => {
                for device in 0..DEVICES {
                    let _ = self.devices[device].syncer.sync().await;
                }
            }
            Step::Flake { device, fault } => {
                let remote = self.devices[device].syncer.remote();
                match fault {
                    Flake::FailUpload => remote.inject(RemoteOp::Put, None, Fault::Fail, 1),
                    Flake::LoseUploadReply => {
                        remote.inject(RemoteOp::Put, None, Fault::LoseReply, 1)
                    }
                    Flake::UploadAppliedThenRefused => {
                        remote.inject(RemoteOp::Put, None, Fault::AppliedThenRefused, 1)
                    }
                    Flake::FailDownload => remote.inject(RemoteOp::Get, None, Fault::Fail, 1),
                    Flake::FailListing => remote.inject(RemoteOp::List, None, Fault::Fail, 1),
                    Flake::FailDelete => remote.inject(RemoteOp::Delete, None, Fault::Fail, 1),
                }
            }
        }
    }

    /// Syncs everything until a whole round does nothing. Panics if it never settles.
    async fn settle(&mut self) {
        for _round in 0..20 {
            let mut busy = false;
            for device in 0..DEVICES {
                match self.devices[device].syncer.sync().await {
                    Ok(report) => busy |= !report.is_quiet(),
                    Err(_) => busy = true,
                }
            }
            if !busy {
                return;
            }
        }
        panic!("the devices never settled");
    }

    /// Every text present anywhere: each device's folder (conflict copies included) and the remote.
    async fn all_texts(&self) -> BTreeSet<String> {
        let mut texts = BTreeSet::new();
        for device in &self.devices {
            let fs = device.syncer.fs();
            for entry in fs.list().unwrap() {
                let plain = self
                    .world
                    .keys
                    .open(&entry.path, &fs.read(&entry.path).unwrap())
                    .unwrap_or_else(|e| panic!("{} does not decrypt: {e}", entry.path));
                texts.insert(String::from_utf8(plain).unwrap());
            }
        }
        for path in self.world.remote_paths().await {
            texts.extend(self.world.remote_plain(&path).await);
        }
        texts
    }

    async fn check(&self) -> Result<(), String> {
        // 1. no write is lost
        let present = self.all_texts().await;
        let lost: Vec<_> = self
            .versions
            .leaves()
            .into_iter()
            .filter(|t| !present.contains(t))
            .collect();
        if !lost.is_empty() {
            return Err(format!("saved versions were lost: {lost:?}"));
        }

        // 2. nothing readable was left anywhere: every object on the remote and every file on every device
        //    is ciphertext (conflict copies included)
        let sealed = |bytes: &[u8]| {
            flint_crypto::is_sealed(
                &bytes[..bytes.len().min(flint_crypto::SNIFF_LEN)],
                bytes.len() as u64,
            )
        };
        for path in self.world.remote_paths().await {
            let object = self.world.remote().get(&p(&path)).await.unwrap();
            if !sealed(&object.bytes) {
                return Err(format!("remote object {path} is not ciphertext"));
            }
        }
        for (i, device) in self.devices.iter().enumerate() {
            let fs = device.syncer.fs();
            for entry in fs.list().unwrap() {
                if !sealed(&fs.read(&entry.path).unwrap()) {
                    return Err(format!("device {i}: {} is not ciphertext", entry.path));
                }
            }
        }

        // 3. devices converge, except where a conflict is deliberately held
        let held: BTreeSet<String> = self
            .devices
            .iter()
            .flat_map(|d| d.syncer.state().conflicts().unwrap())
            .map(|c| c.path.to_string())
            .collect();
        let mut paths: BTreeSet<String> = self.world.remote_paths().await.into_iter().collect();
        for device in &self.devices {
            paths.extend(
                device
                    .syncer
                    .fs()
                    .list()
                    .unwrap()
                    .into_iter()
                    .map(|e| e.path.to_string()),
            );
        }
        for path in paths.iter().filter(|p| !held.contains(*p)) {
            let mut views: Vec<(String, Option<String>)> =
                vec![("remote".into(), self.world.remote_plain(path).await)];
            for (i, device) in self.devices.iter().enumerate() {
                views.push((format!("device {i}"), device.read(path)));
            }
            if views
                .iter()
                .map(|(_, text)| text)
                .collect::<BTreeSet<_>>()
                .len()
                > 1
            {
                return Err(format!("{path} differs after settling: {views:?}"));
            }
        }
        Ok(())
    }
}

async fn run(steps: &[Step]) -> Result<(), String> {
    let mut sim = Sim::new();
    for step in steps {
        sim.apply(step).await;
    }
    sim.settle().await;
    sim.check()
        .await
        .map_err(|e| format!("{e}\nversion graph: {:?}", sim.versions.parents))
}

fn run_blocking(steps: &[Step]) -> Result<(), String> {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(run(steps))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 300, max_shrink_iters: 2000, ..ProptestConfig::default() })]

    #[test]
    fn no_saved_version_is_lost_and_the_devices_converge(steps in proptest::collection::vec(step(), 1..45)) {
        if let Err(problem) = run_blocking(&steps) {
            prop_assert!(false, "{problem}");
        }
    }
}
