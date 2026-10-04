//! Fingerprint unlock. On Android the store's data key is wrapped by a Keystore key that needs a
//! fingerprint for every use; this device then keeps only the wrapped copy. Elsewhere it is not available
//! yet (macOS Touch ID is still to do).

use flint_registry::StoreId;
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::dto::CmdError;
use crate::{Service, Worker};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BiometricState {
    /// This device can do fingerprint unlock (hardware present and a finger enrolled).
    pub available: bool,
    /// This store has a wrapped key, so unlock can ask for the fingerprint instead of the password.
    pub enabled: bool,
}

fn store_id(id: &str) -> Result<StoreId, CmdError> {
    StoreId::parse(id).ok_or_else(|| CmdError::other(format!("{id:?} is not a store id")))
}

#[tauri::command]
pub async fn biometric_state(
    app: AppHandle,
    worker: State<'_, Worker<Service>>,
    id: String,
) -> Result<BiometricState, CmdError> {
    let id = store_id(&id)?;
    Ok(worker
        .call(move |_| Box::pin(async move { platform::state(&app, &id) }))
        .await)
}

/// Wraps the unlocked store's data key under the fingerprint-gated key (asks for a fingerprint).
#[tauri::command]
pub async fn biometric_enable(
    app: AppHandle,
    worker: State<'_, Worker<Service>>,
    id: String,
) -> Result<(), CmdError> {
    let id = store_id(&id)?;
    worker
        .call(move |svc| Box::pin(async move { platform::enable(&app, svc, &id) }))
        .await
}

/// Unlocks the store with a fingerprint.
#[tauri::command]
pub async fn biometric_unlock(
    app: AppHandle,
    worker: State<'_, Worker<Service>>,
    id: String,
) -> Result<(), CmdError> {
    let id = store_id(&id)?;
    worker
        .call(move |svc| Box::pin(async move { platform::unlock(&app, svc, &id) }))
        .await
}

#[tauri::command]
pub async fn biometric_disable(
    app: AppHandle,
    worker: State<'_, Worker<Service>>,
    id: String,
) -> Result<(), CmdError> {
    let id = store_id(&id)?;
    worker
        .call(move |_| Box::pin(async move { platform::disable(&app, &id) }))
        .await
}

#[cfg(target_os = "android")]
mod platform {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use flint_crypto::{Dek, StoreKeys};
    use flint_registry::StoreId;
    use tauri::AppHandle;
    use tauri_plugin_keystore::{DataRequest, KeystoreExt, Sealed};

    use super::BiometricState;
    use crate::Service;
    use crate::dto::CmdError;

    /// `<id>` -> the data key, wrapped. Useless without the fingerprint-gated key.
    fn file(app: &AppHandle) -> PathBuf {
        crate::config_dir(app).join("biometric.json")
    }

    fn read(app: &AppHandle) -> BTreeMap<String, Sealed> {
        std::fs::read_to_string(file(app))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn write(app: &AppHandle, map: &BTreeMap<String, Sealed>) -> Result<(), CmdError> {
        let path = file(app);
        std::fs::create_dir_all(path.parent().expect("a file has a folder"))
            .map_err(|e| CmdError::other(e.to_string()))?;
        std::fs::write(path, serde_json::to_string(map).expect("plain data"))
            .map_err(|e| CmdError::other(e.to_string()))
    }

    fn failure(error: tauri_plugin_keystore::Error) -> CmdError {
        let kind = match error.code() {
            // 10: dismissed, 13: "Use password" tapped, 7 and 9: too many tries
            Some("auth_error_10" | "auth_error_13" | "auth_error_7" | "auth_error_9") => {
                "biometricCancelled"
            }
            Some("key_invalidated") => "keyInvalidated",
            _ => "other",
        };
        CmdError {
            kind,
            message: error.to_string(),
        }
    }

    pub fn state(app: &AppHandle, id: &StoreId) -> BiometricState {
        let available = app
            .keystore()
            .status()
            .is_ok_and(|s| s.biometric == "ready");
        BiometricState {
            available,
            enabled: available && read(app).contains_key(id.as_str()),
        }
    }

    pub fn enable(app: &AppHandle, svc: &Service, id: &StoreId) -> Result<(), CmdError> {
        let keys = svc
            .registry()
            .unlocked_keys(id)?
            .ok_or_else(|| flint_app_service::Error::Locked(id.clone()))?;
        let mut bytes = keys.key_id().to_be_bytes().to_vec();
        bytes.extend_from_slice(keys.dek().as_bytes());
        let wrapped = app
            .keystore()
            .wrap(DataRequest {
                data: STANDARD.encode(&bytes),
            })
            .map_err(failure)?;
        let mut map = read(app);
        map.insert(id.to_string(), wrapped);
        write(app, &map)
    }

    pub fn unlock(app: &AppHandle, svc: &Service, id: &StoreId) -> Result<(), CmdError> {
        let wrapped = read(app)
            .remove(id.as_str())
            .ok_or_else(|| CmdError::other("fingerprint unlock is not set up for this store"))?;
        let opened = match app.keystore().unwrap(wrapped) {
            Ok(opened) => opened,
            Err(e) => {
                let error = failure(e);
                if error.kind == "keyInvalidated" {
                    // A fingerprint was added or removed: every wrapped key is lost, the password is needed.
                    let _ = write(app, &BTreeMap::new());
                    let _ = app.keystore().forget();
                }
                return Err(error);
            }
        };
        let bytes = STANDARD
            .decode(opened.data)
            .map_err(|e| CmdError::other(e.to_string()))?;
        let (key_id, dek) = bytes
            .split_at_checked(4)
            .ok_or_else(|| CmdError::other("the wrapped key is damaged"))?;
        let key_id = <[u8; 4]>::try_from(key_id).expect("4 bytes");
        let dek =
            <[u8; 32]>::try_from(dek).map_err(|_| CmdError::other("the wrapped key is damaged"))?;
        svc.registry().unlock(
            id,
            &StoreKeys::new(u32::from_be_bytes(key_id), Dek::from_bytes(dek)),
        )?;
        Ok(())
    }

    pub fn disable(app: &AppHandle, id: &StoreId) -> Result<(), CmdError> {
        let mut map = read(app);
        if map.remove(id.as_str()).is_some() {
            write(app, &map)?;
        }
        Ok(())
    }
}

#[cfg(not(target_os = "android"))]
mod platform {
    use flint_registry::StoreId;
    use tauri::AppHandle;

    use super::BiometricState;
    use crate::Service;
    use crate::dto::CmdError;

    pub fn state(_: &AppHandle, _: &StoreId) -> BiometricState {
        BiometricState {
            available: false,
            enabled: false,
        }
    }

    pub fn enable(_: &AppHandle, _: &Service, _: &StoreId) -> Result<(), CmdError> {
        Err(CmdError::other(
            "fingerprint unlock is not available on this device",
        ))
    }

    pub fn unlock(_: &AppHandle, _: &Service, _: &StoreId) -> Result<(), CmdError> {
        Err(CmdError::other(
            "fingerprint unlock is not available on this device",
        ))
    }

    pub fn disable(_: &AppHandle, _: &StoreId) -> Result<(), CmdError> {
        Ok(())
    }
}
