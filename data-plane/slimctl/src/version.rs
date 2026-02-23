use std::env::consts::{ARCH, OS};

pub const SEM_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn print_version() {
    println!(
        "Version: sem={} git={} build={} rust={} platform={}/{}",
        SEM_VERSION,
        git_commit(),
        build_date(),
        rustc_version(),
        OS,
        ARCH
    );
}

fn git_commit() -> &'static str {
    option_env!("SLIMCTL_GIT_COMMIT").unwrap_or("unknown")
}

fn build_date() -> &'static str {
    option_env!("SLIMCTL_BUILD_DATE").unwrap_or("unknown")
}

fn rustc_version() -> &'static str {
    option_env!("RUSTC_VERSION").unwrap_or("unknown")
}
