use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

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
