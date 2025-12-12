// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::process::Command;

fn set_env(name: &str, cmd: &mut Command) {
    let value = match cmd.output() {
        Ok(output) => String::from_utf8(output.stdout)
            .unwrap_or_default()
            .trim()
            .to_string(),
        Err(err) => {
            println!("cargo:warning={}", err);
            "unknown".to_string()
        }
    };
    println!("cargo:rustc-env={}={}", name, value);
}

fn main() {
    // Git commit hash (short)
    set_env(
        "GIT_SHA",
        Command::new("git").args(["rev-parse", "--short", "HEAD"]),
    );

    // Build date in ISO 8601 UTC format
    set_env(
        "BUILD_DATE",
        Command::new("date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"]),
    );

    // Build profile (debug/release)
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=PROFILE={profile}");
}
