use flint_core::StorePath;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0} does not exist on the remote")]
    NotFound(StorePath),

    /// A conditional write or delete was refused because the object is not in the state the caller
    /// expected (it exists, changed, or is gone). The caller re-plans; nothing was overwritten.
    #[error("{0} is not in the expected state on the remote (precondition failed)")]
    PreconditionFailed(StorePath),

    /// The server says the bucket does not exist. Usually the wrong *name*: some services show both a bucket
    /// name and a long ID, and the S3 API wants the name.
    #[error(
        "the bucket does not exist: {0} (check that FLINT_S3_BUCKET / the bucket setting is the bucket's name, not its ID)"
    )]
    NoSuchBucket(String),

    /// Wrong credentials, or a bucket policy that forbids the call.
    #[error("access denied by the remote: {0}")]
    AccessDenied(String),

    #[error(
        "refusing to use insecure endpoint {0}: it must be https (plain http is only allowed for localhost)"
    )]
    InsecureEndpoint(String),

    #[error("invalid remote configuration: {0}")]
    InvalidConfig(String),

    /// Anything else the backend reported (network, server error after retries, ...).
    #[error("remote error: {0}")]
    Backend(#[source] object_store::Error),
}

/// S3 error codes that mean "your credentials or permissions are the problem".
const ACCESS_CODES: &[&str] = &[
    "SignatureDoesNotMatch",
    "InvalidAccessKeyId",
    "AccessDenied",
    "AllAccessDisabled",
    "ExpiredToken",
    "InvalidToken",
    "RequestTimeTooSkewed",
];

/// `object_store` reports some failures (a refused `list`, for one) as a generic error even when the
/// server answered 401 or 403, so look at the response itself.
fn is_access_failure(error: &object_store::Error) -> bool {
    let text = format!("{error:?}");
    text.contains("status: 403")
        || text.contains("status: 401")
        || ACCESS_CODES.iter().any(|code| text.contains(code))
}

/// A 404 can mean "no such object" or "no such bucket"; only the S3 error code in the response says which.
pub(crate) fn is_missing_bucket(error: &object_store::Error) -> bool {
    format!("{error:?}").contains("NoSuchBucket")
}

impl Error {
    /// Classifies a backend error that is not about one particular object (a failed listing).
    pub(crate) fn from_backend_listing(error: object_store::Error) -> Self {
        if is_missing_bucket(&error) {
            Self::NoSuchBucket(error.to_string())
        } else if is_access_failure(&error) {
            Self::AccessDenied(error.to_string())
        } else {
            Self::Backend(error)
        }
    }

    /// Classifies a backend error that happened while working on `path`.
    pub(crate) fn from_backend(path: &StorePath, error: object_store::Error) -> Self {
        if is_missing_bucket(&error) {
            return Self::NoSuchBucket(error.to_string());
        }
        match error {
            object_store::Error::NotFound { .. } => Self::NotFound(path.clone()),
            object_store::Error::AlreadyExists { .. }
            | object_store::Error::Precondition { .. }
            | object_store::Error::NotModified { .. } => Self::PreconditionFailed(path.clone()),
            object_store::Error::PermissionDenied { .. }
            | object_store::Error::Unauthenticated { .. } => Self::AccessDenied(error.to_string()),
            other if is_access_failure(&other) => Self::AccessDenied(other.to_string()),
            other => Self::Backend(other),
        }
    }
}
