//! Embeds the helper APK in the binary when `AU_EMBED_APK` names one (release builds), so `au`
//! alone is enough to set up a device. Dev builds embed nothing and look for `aubridge.apk` beside
//! the binary instead. The APK's SHA-256 is compiled in and checked again before every install.
use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=AU_EMBED_APK");
    println!("cargo:rustc-check-cfg=cfg(embedded_apk)");
    let Some(path) = env::var_os("AU_EMBED_APK").map(PathBuf::from) else { return };
    println!("cargo:rerun-if-changed={}", path.display());
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("AU_EMBED_APK {} is unreadable: {e}", path.display()));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("aubridge.apk");
    fs::write(out, &bytes).expect("write the embedded APK");
    let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    println!("cargo:rustc-env=AU_APK_SHA256={hash}");
    println!("cargo:rustc-cfg=embedded_apk");
}
