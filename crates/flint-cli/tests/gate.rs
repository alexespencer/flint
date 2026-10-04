//! The M4 gate, as a script: two `flint` instances (own config folder, own secrets, own store folder)
//! sharing one bucket, driven through the same argument parsing and output as the real binary.

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use flint_app_service::{AppService, Error as ServiceError, InMemoryConnect};
use flint_cli::{Cli, Env, Error, Input, StdinInput, run};
use flint_crypto::KdfParams;
use flint_registry::{MemorySecrets, Registry};
use flint_sync::DeviceName;
use tempfile::TempDir;

const FAST: KdfParams = KdfParams {
    m_kib: 8,
    t: 1,
    p: 1,
};

struct Scripted(VecDeque<String>);

impl Input for Scripted {
    fn secret(&mut self, prompt: &str) -> io::Result<String> {
        self.0
            .pop_front()
            .ok_or_else(|| io::Error::other(format!("the script has no answer for {prompt:?}")))
    }
}

struct Instance {
    env_service: Option<AppService<MemorySecrets, InMemoryConnect>>,
    folder: TempDir,
    _config: TempDir,
}

impl Instance {
    fn new(device: &str, bucket: &InMemoryConnect) -> Self {
        let config = TempDir::new().unwrap();
        let registry = Registry::open(config.path(), MemorySecrets::new()).unwrap();
        let service = AppService::new(registry, bucket.clone(), DeviceName::new(device))
            .with_kdf(FAST)
            .with_quiet_period(Duration::ZERO);
        Self {
            env_service: Some(service),
            folder: TempDir::new().unwrap(),
            _config: config,
        }
    }

    fn dir(&self) -> PathBuf {
        self.folder.path().join("Notes")
    }

    /// Runs `flint <args>` answering prompts from `answers`; returns what it printed.
    async fn flint(&mut self, args: &str, answers: &[&str]) -> Result<String, Error> {
        let mut argv = vec!["flint".to_owned()];
        argv.extend(args.split_whitespace().map(str::to_owned));
        let cli = Cli::try_parse_from(argv).unwrap();
        let mut input = Scripted(answers.iter().map(|s| s.to_string()).collect());
        let mut out = Vec::new();
        let mut env = Env {
            service: self.env_service.take().unwrap(),
            input: &mut input,
            out: &mut out,
        };
        let result = run(cli, &mut env).await;
        self.env_service = Some(env.service);
        result.map(|()| String::from_utf8(out).unwrap())
    }

    fn remote_flags(&self) -> String {
        format!(
            "--bucket b --prefix vault --region auto --access-key-id k --folder {}",
            self.dir().display()
        )
    }

    fn write(&self, path: &str, text: &str) {
        let file = self.dir().join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }
}

#[tokio::test]
async fn two_instances_share_a_store_surface_a_conflict_and_survive_a_password_change() {
    let bucket = InMemoryConnect::new();

    // the first device creates the store from a folder of existing notes
    let mut mac = Instance::new("Mac", &bucket);
    mac.write("Plan.md", "# Plan\nsee [[Ideas]]");
    mac.write("Ideas.md", "ideas");
    let created = mac
        .flint(
            &format!("create-store Notes {}", mac.remote_flags()),
            &["s3-secret", "pw", "pw"],
        )
        .await
        .unwrap();
    assert!(created.contains("created store"), "{created}");
    assert!(created.contains("2 uploaded"), "{created}");

    // the second joins with only the bucket details and the password
    let mut phone = Instance::new("Phone", &bucket);
    let joined = phone
        .flint(
            &format!("join Notes {}", phone.remote_flags()),
            &["s3-secret", "pw"],
        )
        .await
        .unwrap();
    assert!(joined.contains("2 downloaded"), "{joined}");
    assert_eq!(fs::read_dir(phone.dir()).unwrap().count(), 3); // two notes and .store

    // both edit the same note: the second to sync sees a conflict
    mac.write("Plan.md", "mac edit"); // dropped in as plaintext, encrypted by the next sync
    phone.write("Plan.md", "phone edit");
    assert!(mac.flint("sync", &[]).await.unwrap().contains("1 uploaded"));
    let conflicted = phone.flint("sync", &[]).await.unwrap();
    assert!(conflicted.contains("CONFLICT Plan.md"), "{conflicted}");
    let status = phone.flint("status", &[]).await.unwrap();
    assert!(
        status.contains("CONFLICT Plan.md (other version: Plan.conflict-"),
        "{status}"
    );

    // merging and resolving clears it, and the mac ends up with the merge
    let copy = fs::read_dir(phone.dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .find(|n| n.contains(".conflict-"))
        .unwrap();
    fs::remove_file(phone.dir().join(&copy)).unwrap();
    phone.write("Plan.md", "merged");
    phone.flint("resolve Plan.md", &[]).await.unwrap();
    phone.flint("sync", &[]).await.unwrap();
    mac.flint("sync", &[]).await.unwrap();
    assert!(!mac.flint("status", &[]).await.unwrap().contains("CONFLICT"));

    // a password change on one keeps the other working
    let changed = mac.flint("passwd", &["pw", "new", "new"]).await.unwrap();
    assert!(changed.contains("password changed"), "{changed}");
    phone.write("Later.md", "written after the change");
    let synced = phone.flint("sync", &[]).await.unwrap();
    assert!(
        synced.contains("1 uploaded") && !synced.contains("FAILED"),
        "{synced}"
    );
    phone.flint("lock", &[]).await.unwrap();
    assert!(matches!(
        phone.flint("unlock", &["pw"]).await,
        Err(Error::Service(ServiceError::WrongPassword))
    ));
    phone.flint("unlock", &["new"]).await.unwrap();
    assert!(
        mac.flint("sync", &[])
            .await
            .unwrap()
            .contains("1 downloaded")
    );
}

#[tokio::test]
async fn mistakes_are_reported_plainly() {
    let bucket = InMemoryConnect::new();
    let mut mac = Instance::new("Mac", &bucket);

    assert!(matches!(
        mac.flint("sync", &[]).await,
        Err(Error::NoActiveStore)
    ));
    let args = format!("create-store Notes {}", mac.remote_flags());
    assert!(matches!(
        mac.flint(&args, &["secret", "one", "two"]).await,
        Err(Error::PasswordsDiffer)
    ));
    assert!(
        mac.flint("stores", &[])
            .await
            .unwrap()
            .contains("no stores yet")
    );

    mac.flint(&args, &["secret", "pw", "pw"]).await.unwrap();
    assert!(matches!(
        mac.flint("--store Nope sync", &[]).await,
        Err(Error::NoSuchStore(_))
    ));
    mac.flint("lock", &[]).await.unwrap();
    assert!(matches!(
        mac.flint("sync", &[]).await,
        Err(Error::Service(ServiceError::Locked(_)))
    ));
    let listing = mac.flint("stores", &[]).await.unwrap();
    assert!(
        listing.contains("Notes (active)") && listing.contains("locked"),
        "{listing}"
    );

    mac.flint("forget Notes", &[]).await.unwrap();
    assert!(
        mac.flint("stores", &[])
            .await
            .unwrap()
            .contains("no stores yet")
    );
}

#[test]
fn secrets_can_come_from_standard_input_one_per_line() {
    let mut input = StdinInput(io::Cursor::new("first\r\nsecond\nlast"));
    assert_eq!(input.secret("a").unwrap(), "first");
    assert_eq!(input.secret("b").unwrap(), "second");
    assert_eq!(input.secret("c").unwrap(), "last");
    assert!(input.secret("d").is_err());
}
