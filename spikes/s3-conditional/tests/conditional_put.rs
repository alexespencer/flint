//! M0 spike: does object_store's S3 backend do ETag-conditional writes against s3s-fs?
//! Confirms: If-None-Match (Create), If-Match (Update), 412 on a stale ETag,
//! user-metadata round-trip, and ETag stability across HEAD/GET/LIST.

use std::net::SocketAddr;

use bytes::Bytes;
use futures_util_shim::collect_list;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use object_store::aws::{AmazonS3Builder, S3ConditionalPut};
use object_store::path::Path;
use object_store::{
    Attribute, AttributeValue, Attributes, Error, ObjectStore, ObjectStoreExt, PutMode, PutOptions, PutPayload,
    UpdateVersion,
};
use s3s::auth::SimpleAuth;
use s3s::service::S3ServiceBuilder;
use s3s_fs::FileSystem;
use tokio::net::TcpListener;

const BUCKET: &str = "flint-test";

/// Minimal stand-in for futures::TryStreamExt so we don't add the dependency.
mod futures_util_shim {
    use object_store::{path::Path, ObjectMeta, ObjectStore};
    pub async fn collect_list(store: &dyn ObjectStore, prefix: Option<&Path>) -> Vec<ObjectMeta> {
        use futures_core_shim::next;
        let mut s = store.list(prefix);
        let mut out = vec![];
        while let Some(m) = next(&mut s).await {
            out.push(m.unwrap());
        }
        out
    }
    mod futures_core_shim {
        use std::pin::Pin;
        use std::task::{Context, Poll};
        pub async fn next<S: futures_core::Stream + Unpin>(s: &mut S) -> Option<S::Item> {
            std::future::poll_fn(|cx: &mut Context<'_>| -> Poll<Option<S::Item>> {
                Pin::new(&mut *s).poll_next(cx)
            })
            .await
        }
    }
}

struct Emulator {
    addr: SocketAddr,
    _dir: tempfile::TempDir,
}

async fn spawn_s3s_fs() -> Emulator {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(BUCKET)).unwrap();
    let fs = FileSystem::new(dir.path()).unwrap();
    let service = {
        let mut b = S3ServiceBuilder::new(fs);
        b.set_auth(SimpleAuth::from_single("test-key", "test-secret"));
        b.build()
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (sock, _) = listener.accept().await.unwrap();
            let svc = service.clone();
            tokio::spawn(async move {
                let _ = ConnBuilder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(sock), svc)
                    .await;
            });
        }
    });
    Emulator { addr, _dir: dir }
}

fn store(addr: SocketAddr, conditional: bool) -> impl ObjectStore {
    let mode = if conditional { S3ConditionalPut::ETagMatch } else { S3ConditionalPut::Disabled };
    let mut b = AmazonS3Builder::new()
        .with_endpoint(format!("http://{addr}"))
        .with_allow_http(true)
        .with_region("us-east-1")
        .with_bucket_name(BUCKET)
        .with_access_key_id("test-key")
        .with_secret_access_key("test-secret")
        .with_virtual_hosted_style_request(false);
    b = b.with_conditional_put(mode);
    b.build().unwrap()
}

fn update_of(e_tag: &Option<String>) -> PutMode {
    PutMode::Update(UpdateVersion { e_tag: e_tag.clone(), version: None })
}

#[tokio::test]
async fn create_is_exclusive() {
    let emu = spawn_s3s_fs().await;
    let s = store(emu.addr, true);
    let p = Path::from("notes/a.md");

    s.put_opts(&p, PutPayload::from("one"), PutMode::Create.into()).await.unwrap();
    let err = s
        .put_opts(&p, PutPayload::from("two"), PutMode::Create.into())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::AlreadyExists { .. }), "got {err:?}");
    assert_eq!(s.get(&p).await.unwrap().bytes().await.unwrap(), Bytes::from("one"));
}

#[tokio::test]
async fn update_with_current_etag_succeeds_and_stale_etag_is_412() {
    let emu = spawn_s3s_fs().await;
    let s = store(emu.addr, true);
    let p = Path::from("notes/a.md");

    let v1 = s.put(&p, PutPayload::from("v1")).await.unwrap();
    assert!(v1.e_tag.is_some());

    // Device A updates with the ETag it last saw.
    let v2 = s
        .put_opts(&p, PutPayload::from("v2"), PutOptions::from(update_of(&v1.e_tag)))
        .await
        .unwrap();
    assert_ne!(v1.e_tag, v2.e_tag);

    // Device B still holds v1's ETag: must be refused, and must not clobber v2.
    let err = s
        .put_opts(&p, PutPayload::from("b-edit"), PutOptions::from(update_of(&v1.e_tag)))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Precondition { .. }), "expected 412, got {err:?}");
    assert_eq!(s.get(&p).await.unwrap().bytes().await.unwrap(), Bytes::from("v2"));
}

#[tokio::test]
async fn update_is_not_implemented_when_conditional_put_disabled() {
    let emu = spawn_s3s_fs().await;
    let s = store(emu.addr, false);
    let p = Path::from("a.md");
    let v1 = s.put(&p, PutPayload::from("v1")).await.unwrap();
    let err = s
        .put_opts(&p, PutPayload::from("v2"), PutOptions::from(update_of(&v1.e_tag)))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotImplemented { .. }), "got {err:?}");
}

#[tokio::test]
async fn user_metadata_round_trips_and_etag_is_stable() {
    let emu = spawn_s3s_fs().await;
    let s = store(emu.addr, true);
    let p = Path::from("notes/b.md");
    let sha = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

    let mut attrs = Attributes::new();
    attrs.insert(
        Attribute::Metadata("sha256".into()),
        AttributeValue::from(sha.to_string()),
    );
    let put = s
        .put_opts(&p, PutPayload::from("ciphertext"), PutOptions { attributes: attrs, ..Default::default() })
        .await
        .unwrap();

    let got = s.get(&p).await.unwrap();
    let meta_etag = got.meta.e_tag.clone();
    let value = got.attributes.get(&Attribute::Metadata("sha256".into())).map(|v| v.to_string());
    assert_eq!(value.as_deref(), Some(sha), "attributes: {:?}", got.attributes);

    let head = s.head(&p).await.unwrap();
    let listed = collect_list(&s, Some(&Path::from("notes"))).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(put.e_tag, meta_etag);
    assert_eq!(put.e_tag, head.e_tag);
    // FINDING: s3s-fs omits the ETag from LIST responses (real S3 includes it), so the
    // sync planner must not depend on LIST ETags when running against this emulator.
    assert_eq!(listed[0].e_tag, None, "s3s-fs now returns ETags in LIST; drop the HEAD fallback");
    assert_eq!(listed[0].size, "ciphertext".len() as u64);
}
