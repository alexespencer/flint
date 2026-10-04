use serde::{Serialize, Serializer};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("the keystore plugin only works on Android")]
    Unsupported,

    #[cfg(mobile)]
    #[error(transparent)]
    PluginInvoke(#[from] tauri::plugin::mobile::PluginInvokeError),
}

impl Error {
    /// The Kotlin side's reason code when it refused: `auth_error_10` (cancelled), `auth_error_13`
    /// ("Use password" tapped), `auth_error_7`/`_9` (locked out), `key_invalidated` (a fingerprint was
    /// enrolled or removed), and so on.
    #[cfg(mobile)]
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::PluginInvoke(tauri::plugin::mobile::PluginInvokeError::InvokeRejected(r)) => {
                r.code.as_deref()
            }
            _ => None,
        }
    }

    #[cfg(not(mobile))]
    pub fn code(&self) -> Option<&str> {
        None
    }
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
