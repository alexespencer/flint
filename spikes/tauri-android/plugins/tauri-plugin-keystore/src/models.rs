use serde::{Deserialize, Serialize};

/// What the device and the stored key look like. `biometric` is "ready", "none_enrolled",
/// "no_hardware", "hw_unavailable", "security_update_required" or "unsupported".
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
  pub biometric: String,
  pub sdk: u32,
  pub key_exists: bool,
  pub inside_secure_hardware: Option<bool>,
  /// "strongbox", "tee" or "software" (Android 12+ only).
  pub security_level: Option<String>,
  pub user_authentication_required: Option<bool>,
  pub invalidated_by_biometric_enrollment: Option<bool>,
}

/// Base64 of the bytes to protect (a store's data key).
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WrapRequest {
  pub data: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Wrapped {
  pub iv: String,
  pub ciphertext: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnwrapRequest {
  pub iv: String,
  pub ciphertext: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unwrapped {
  pub data: String,
}
