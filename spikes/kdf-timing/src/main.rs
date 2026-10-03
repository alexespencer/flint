//! M0 spike: time Argon2id at several parameter sets so we can pick KDF params that unlock
//! in roughly 0.5-1 s on the slowest target (the phone). Run on each device and compare.
//!
//!     cargo run --release -p spike-kdf-timing
//!
//! On Android, cross-compile and `adb push` + `adb shell` it (see docs/m0-findings.md).

use std::time::Instant;

use argon2::{Algorithm, Argon2, Params, Version};

const RUNS: usize = 5;

fn median_ms(m_kib: u32, t: u32, p: u32) -> f64 {
    let params = Params::new(m_kib, t, p, Some(32)).unwrap();
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let salt = [7u8; 16];
    let mut out = [0u8; 32];
    let mut times: Vec<f64> = (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            argon.hash_password_into(b"correct horse battery staple", &salt, &mut out).unwrap();
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    times[RUNS / 2]
}

fn main() {
    println!("target: {}-{}", std::env::consts::OS, std::env::consts::ARCH);
    println!("{:>10} {:>3} {:>3} {:>10}", "mem MiB", "t", "p", "median ms");
    // OWASP's minimum is m=19 MiB,t=2,p=1; RFC 9106 second recommendation is m=64 MiB,t=3,p=4.
    for (mib, t, p) in [(19, 2, 1), (32, 3, 1), (64, 2, 1), (64, 3, 1), (64, 3, 4), (128, 3, 1), (256, 2, 1)] {
        println!("{mib:>10} {t:>3} {p:>3} {:>10.0}", median_ms(mib * 1024, t, p));
    }
}
