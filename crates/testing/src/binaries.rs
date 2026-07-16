use std::path::{Path, PathBuf};
use std::process::Command;

/// Workspace root (`slim/`), two levels above this crate manifest dir
/// (`slim/crates/testing`).
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn abs_path(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn slim_in_profile_dir(profile_dir: &Path) -> PathBuf {
    binary_in_profile_dir(profile_dir, "slim")
}

fn sdk_mock_in_profile_dir(profile_dir: &Path) -> PathBuf {
    binary_in_profile_dir(profile_dir, "sdk-mock")
}

fn binary_in_profile_dir(profile_dir: &Path, name: &str) -> PathBuf {
    abs_path(&profile_dir.join(name))
}

fn binary_from_current_exe(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let profile_dir = exe.parent()?.parent()?;
    Some(binary_in_profile_dir(profile_dir, name))
}

fn resolve_binary(env_var: &str, file_name: &str, default_path: PathBuf) -> PathBuf {
    if let Ok(path) = std::env::var(env_var) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return abs_path(&path);
        }
    }

    let cargo_exe_var = format!("CARGO_BIN_EXE_{}", file_name.replace('-', "_"));
    if let Ok(path) = std::env::var(&cargo_exe_var) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return path;
        }
    }

    if let Some(path) = binary_from_current_exe(file_name)
        && path.is_file()
    {
        return path;
    }

    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        for profile in ["debug", "release"] {
            let candidate =
                binary_in_profile_dir(&PathBuf::from(&target_dir).join(profile), file_name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }

    let workspace = workspace_root();
    let default_target = binary_in_profile_dir(&workspace.join("target/debug"), file_name);
    if default_target.is_file() {
        return default_target;
    }

    if let Ok(target) = rustc_host_target() {
        let triple_target = binary_in_profile_dir(
            &workspace.join("target").join(target).join("debug"),
            file_name,
        );
        if triple_target.is_file() {
            return triple_target;
        }
    }
    default_path
}

/// Path to the `slim` binary.
///
/// Resolution order:
/// 1. `SLIM_BINARY` env override
/// 2. `CARGO_BIN_EXE_slim` when set by Cargo artifact deps
/// 3. Same Cargo target profile as the running test binary (`…/target/<profile>/slim`)
/// 4. `CARGO_TARGET_DIR/<profile>/slim`
/// 5. `target/debug/slim` under the workspace root
/// 6. `target/<host-triple>/debug/slim`
pub fn slim_path() -> PathBuf {
    resolve_binary(
        "SLIM_BINARY",
        "slim",
        slim_in_profile_dir(&workspace_root().join("target/debug")),
    )
}

/// Path to the `sdk-mock` binary.
pub fn sdk_mock_path() -> PathBuf {
    resolve_binary(
        "SDK_MOCK_BINARY",
        "sdk-mock",
        sdk_mock_in_profile_dir(&workspace_root().join("target/debug")),
    )
}

/// Path to the `client` example binary.
pub fn client_path() -> PathBuf {
    resolve_binary(
        "CLIENT_BINARY",
        "client",
        binary_in_profile_dir(&workspace_root().join("target/debug"), "client"),
    )
}

/// Path to the `channel-manager` binary.
pub fn channel_manager_path() -> PathBuf {
    resolve_binary(
        "CHANNEL_MANAGER_BINARY",
        "channel-manager",
        binary_in_profile_dir(&workspace_root().join("target/debug"), "channel-manager"),
    )
}

/// Path to the `slimctl` binary.
pub fn slimctl_path() -> PathBuf {
    resolve_binary(
        "SLIMCTL_BINARY",
        "slimctl",
        binary_in_profile_dir(&workspace_root().join("target/debug"), "slimctl"),
    )
}

/// Path to the `slim-control-plane` binary.
pub fn control_plane_path() -> PathBuf {
    resolve_binary(
        "CONTROL_PLANE_BINARY",
        "slim-control-plane",
        binary_in_profile_dir(&workspace_root().join("target/debug"), "slim-control-plane"),
    )
}

/// Return the path to the built `slim` binary, panicking with build instructions if missing.
pub fn require_slim_binary() -> PathBuf {
    require_binary(slim_path(), "slim", "cargo build --bin slim -p agntcy-slim")
}

/// Return the path to the built `sdk-mock` binary, panicking with build instructions if missing.
pub fn require_sdk_mock_binary() -> PathBuf {
    require_binary(
        sdk_mock_path(),
        "sdk-mock",
        "cargo build --bin sdk-mock -p slim-examples",
    )
}

/// Return the path to the built `client` binary, panicking with build instructions if missing.
pub fn require_client_binary() -> PathBuf {
    require_binary(
        client_path(),
        "client",
        "cargo build --bin client -p slim-examples",
    )
}

/// Return the path to the built `channel-manager` binary, panicking with build instructions if missing.
pub fn require_channel_manager_binary() -> PathBuf {
    require_binary(
        channel_manager_path(),
        "channel-manager",
        "cargo build --bin channel-manager -p agntcy-slim-channel-manager",
    )
}

/// Return the path to the built `slimctl` binary, panicking with build instructions if missing.
pub fn require_slimctl_binary() -> PathBuf {
    require_binary(
        slimctl_path(),
        "slimctl",
        "cargo build --bin slimctl -p agntcy-slimctl",
    )
}

/// Return the path to the built `slim-control-plane` binary, panicking with build instructions if missing.
pub fn require_control_plane_binary() -> PathBuf {
    require_binary(
        control_plane_path(),
        "slim-control-plane",
        "cargo build --bin slim-control-plane -p agntcy-slim-control-plane",
    )
}

fn require_binary(path: PathBuf, name: &str, build_hint: &str) -> PathBuf {
    if path.is_file() {
        return path;
    }

    panic!(
        "{name} binary not found at {}.\n\
         Build it first: {build_hint}\n\
         Or run: task -d crates/testing tests:integration",
        path.display()
    );
}

fn rustc_host_target() -> Result<String, String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }

    output
        .stdout
        .split(|b| *b == b'\n')
        .find_map(|line| {
            let line = std::str::from_utf8(line).ok()?;
            line.strip_prefix("host: ").map(str::trim)
        })
        .map(str::to_string)
        .ok_or_else(|| "failed to parse rustc host target".to_string())
}
