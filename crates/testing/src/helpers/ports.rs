use std::net::TcpListener;
use std::process;

use parking_lot::Mutex;

static NEXT_PORT: Mutex<Option<u16>> = Mutex::new(None);

/// Starting port for this integration-test binary.
///
/// Cargo runs each `tests/*.rs` file as a separate process in parallel. A
/// process-local counter alone is not enough because many processes share the
/// same `pid % 100` bucket. We therefore carve out a 100-port block per test
/// binary (derived from the executable name) and add a small PID offset inside
/// that block, mirroring the Go suite's `(parallel_node, pid)` spacing.
fn port_base() -> u16 {
    let pid_offset = (process::id() % 100) as u16;
    let exe_seed = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .map(|stem| {
            stem.bytes().fold(0u32, |hash, byte| {
                hash.wrapping_mul(31).wrapping_add(u32::from(byte))
            })
        })
        .unwrap_or(0);

    // 100-port blocks keep parallel test binaries from overlapping.
    let block = (exe_seed % 350) * 100;
    20000 + block as u16 + pid_offset
}

/// Returns a free TCP port on 127.0.0.1, using a process-local counter so
/// parallel tests within the same binary are less likely to collide.
pub fn reserve_port() -> u16 {
    let mut next = NEXT_PORT.lock();

    if next.is_none() {
        *next = Some(port_base());
    }

    loop {
        let port = next.expect("initialized above");
        *next = Some(if port >= 65000 { port_base() } else { port + 1 });

        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_base_stays_in_valid_range() {
        let base = port_base();
        assert!((20000..65000).contains(&base));
    }
}
