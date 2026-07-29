use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn new_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}{unique}"));
    fs::create_dir_all(&dir).expect("failed to create temp dir");
    dir
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

pub fn write_temp_config_near_source(
    src_path: &Path,
    pattern: &str,
    replacements: &HashMap<String, String>,
) -> PathBuf {
    let mut contents = fs::read_to_string(src_path).expect("failed to read source config");

    for (old, new) in replacements {
        contents = contents.replace(old, new);
    }

    let parent = src_path
        .parent()
        .expect("source config path must have a parent directory");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX epoch")
        .as_nanos();
    let file_path = parent.join(format!("{pattern}{unique}"));

    fs::write(&file_path, contents).expect("failed to write temp config");
    file_path
}
