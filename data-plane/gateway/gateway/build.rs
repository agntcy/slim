// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use std::process::Command;

fn set_env(name: &str, cmd: &mut Command) {
    let value = match cmd.output() {
        Ok(output) => String::from_utf8(output.stdout).unwrap(),
        Err(err) => {
            println!("cargo:warning={}", err);
            "".to_string()
        }
    };
    println!("cargo:rustc-env={}={}", name, value);
}

fn version() -> String {
    if let Ok(v) = std::env::var("VERSION") {
        if !v.is_empty() {
            return v;
        }
    }

    "0.1.0-dev".to_string()
}

fn main() {
    set_env(
        "GIT_SHA",
        Command::new("git").args(["rev-parse", "--short", "HEAD"]),
    );

    // Capture the ISO 8601 formatted UTC time.
    set_env(
        "BUILD_DATE",
        Command::new("date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"]),
    );

    println!("cargo:rustc-env=VERSION={}", version());

    let profile = std::env::var("PROFILE").expect("PROFILE must be set");
    println!("cargo:rustc-env=PROFILE={profile}");
}
