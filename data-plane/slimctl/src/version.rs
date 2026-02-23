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

#[cfg(test)]
mod tests {
    use super::{SEM_VERSION, build_date, git_commit, rustc_version};

    #[test]
    fn version_strings_are_non_empty() {
        assert!(!SEM_VERSION.is_empty());
        assert!(!git_commit().is_empty());
        assert!(!build_date().is_empty());
        assert!(!rustc_version().is_empty());
    }
}
