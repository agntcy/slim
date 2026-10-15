use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::Ed25519KeyPair;
use tempfile::TempDir;

/// Create a uniquely named temporary directory using the OS temp API.
///
/// The returned [`TempDir`] owns the directory and removes it (and its
/// contents) when dropped, so callers must keep the handle alive for as long
/// as the spawned processes need the files inside it. Use [`TempDir::path`] to
/// obtain the directory path.
pub fn new_temp_dir(prefix: &str) -> TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .expect("failed to create temp dir")
}

/// Write a fresh Ed25519 PKCS#8 PEM key into `dir` and return its path.
///
/// `slimctl cm` signs every channel change with a key, and a channel it
/// creates is owned by that key's `did:key`, so one key per test both owns the
/// channel and signs the grants for it.
pub fn write_signing_key(dir: &Path, name: &str) -> PathBuf {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
        .expect("failed to generate an Ed25519 key");
    let path = dir.join(name);
    fs::write(
        &path,
        slim_auth::utils::bytes_to_pem(
            pkcs8.as_ref(),
            "-----BEGIN PRIVATE KEY-----\n",
            "\n-----END PRIVATE KEY-----\n",
        ),
    )
    .expect("failed to write signing key");
    path
}

pub fn write_temp_config(
    dir: &Path,
    src_path: &Path,
    dst_name: &str,
    replacements: &HashMap<String, String>,
) -> PathBuf {
    let mut contents = fs::read_to_string(src_path).expect("failed to read source config");
    for (old, new) in replacements {
        contents = contents.replace(old, new);
    }

    let dst_path = dir.join(dst_name);
    fs::write(&dst_path, contents).expect("failed to write temp config");
    dst_path
}
