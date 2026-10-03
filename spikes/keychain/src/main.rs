//! M0 spike: unsigned, free macOS secret storage with a Touch ID gate.
//!
//! The data key sits in an ordinary login-keychain item (no access control, no entitlements, no
//! provisioning profile). Before reading it the app asks LocalAuthentication for Touch ID, falling
//! back to the Mac password. The OS does not enforce the gate; the app does.
//!
//!     cargo run -p spike-keychain -- store     # put a fake 32-byte key in the login keychain
//!     cargo run -p spike-keychain -- unlock    # Touch ID prompt, then read the key
//!     cargo run -p spike-keychain -- forget    # delete it

#[cfg(target_os = "macos")]
fn main() {
    use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};

    const SERVICE: &str = "dev.flint.spike";
    const ACCOUNT: &str = "store-dek";

    match std::env::args().nth(1).as_deref() {
        Some("store") => match set_generic_password(SERVICE, ACCOUNT, &[0xAB; 32]) {
            Ok(()) => println!("store: ok"),
            Err(e) => println!("store: FAILED OSStatus {} ({e})", e.code()),
        },
        Some("unlock") => match touch_id("unlock your Flint store") {
            Err(why) => println!("unlock: refused ({why}); key not read"),
            Ok(()) => match get_generic_password(SERVICE, ACCOUNT) {
                Ok(k) => println!("unlock: ok after Touch ID ({} bytes, first byte {:#04x})", k.len(), k[0]),
                Err(e) => println!("unlock: keychain read FAILED OSStatus {} ({e})", e.code()),
            },
        },
        Some("forget") => match delete_generic_password(SERVICE, ACCOUNT) {
            Ok(()) => println!("forget: ok"),
            Err(e) => println!("forget: FAILED OSStatus {} ({e})", e.code()),
        },
        _ => eprintln!("usage: spike-keychain store|unlock|forget"),
    }
}

/// Blocks until the user passes Touch ID (or the Mac password fallback) or cancels.
#[cfg(target_os = "macos")]
fn touch_id(reason: &str) -> Result<(), String> {
    use std::sync::mpsc;
    use std::time::Duration;

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAPolicy};

    let ctx = unsafe { LAContext::new() };
    unsafe { ctx.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthentication) }
        .map_err(|e| format!("policy unavailable: {}", e.localizedDescription()))?;

    let (tx, rx) = mpsc::channel();
    let reply = RcBlock::new(move |ok: Bool, err: *mut NSError| {
        let outcome = if ok.as_bool() {
            Ok(())
        } else {
            Err(unsafe { err.as_ref() }.map_or("unknown".into(), |e| e.localizedDescription().to_string()))
        };
        let _ = tx.send(outcome);
    });
    let reason = NSString::from_str(reason);
    unsafe { ctx.evaluatePolicy_localizedReason_reply(LAPolicy::DeviceOwnerAuthentication, &reason, &reply) };
    rx.recv_timeout(Duration::from_secs(60)).map_err(|_| "timed out".to_string())?
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("macOS only; the Android half of this spike is a Keystore plugin (see docs/m0-findings.md)");
}
