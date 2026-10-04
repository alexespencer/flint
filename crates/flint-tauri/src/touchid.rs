//! Touch ID on macOS through LocalAuthentication, with the Mac password as the fallback.
//!
//! This is a gate the app applies before it reads a key, not something the OS enforces: the key sits in
//! the ordinary login keychain (no signing, no entitlement, no paid developer account; see
//! `docs/m0-findings.md`). It keeps out someone who sits down at an unlocked Mac with the app closed, not
//! code running as the same user.
//!
//! The one `unsafe` in the app lives here: the Objective-C calls are `unsafe` in `objc2`.
#![allow(unsafe_code)]

use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::{NSError, NSString};
use objc2_local_authentication::{LAContext, LAPolicy};

/// How long to wait for the person at the prompt.
const PROMPT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug)]
pub enum Refusal {
    /// They dismissed the prompt, or chose to use the password instead.
    Cancelled,
    Failed(String),
}

/// This Mac has Touch ID set up (hardware present and a finger enrolled).
pub fn available() -> bool {
    // SAFETY: a fresh context, used on this thread only.
    let context = unsafe { LAContext::new() };
    unsafe { context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthenticationWithBiometrics) }
        .is_ok()
}

/// Shows the system prompt (`reason` is its text) and waits. Touch ID, or the Mac password through the
/// prompt's own fallback button, passes. Call from a worker thread: it blocks until the person answers.
pub fn confirm(reason: &str) -> Result<(), Refusal> {
    // SAFETY: a fresh context, used on this thread only.
    let context = unsafe { LAContext::new() };
    unsafe { context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthentication) }
        .map_err(|e| Refusal::Failed(e.localizedDescription().to_string()))?;

    let (answer, reply_received) = mpsc::channel();
    let reply = RcBlock::new(move |ok: Bool, error: *mut NSError| {
        let outcome = if ok.as_bool() {
            Ok(())
        } else {
            // SAFETY: LocalAuthentication passes a valid NSError, or null, for the duration of the call.
            Err(match unsafe { error.as_ref() } {
                // -2 user cancelled, -4 system cancelled, -9 app cancelled, -3 chose the fallback
                Some(e) if matches!(e.code(), -2 | -3 | -4 | -9) => Refusal::Cancelled,
                Some(e) => Refusal::Failed(e.localizedDescription().to_string()),
                None => Refusal::Failed("authentication failed".into()),
            })
        };
        let _ = answer.send(outcome);
    });
    let reason = NSString::from_str(reason);
    // SAFETY: `reply` is kept alive until the answer arrives below.
    unsafe {
        context.evaluatePolicy_localizedReason_reply(
            LAPolicy::DeviceOwnerAuthentication,
            &reason,
            &reply,
        );
    }
    reply_received
        .recv_timeout(PROMPT_TIMEOUT)
        .map_err(|_| Refusal::Failed("timed out waiting for Touch ID".into()))?
}
