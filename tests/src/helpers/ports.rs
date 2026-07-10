use std::net::TcpListener;
use std::process;

use parking_lot::Mutex;

static NEXT_PORT: Mutex<Option<u16>> = Mutex::new(None);

/// Returns a free TCP port on 127.0.0.1, using a process-local counter so
/// parallel test runs are less likely to collide.
pub fn reserve_port() -> u16 {
    let mut next = NEXT_PORT.lock();

    if next.is_none() {
        *next = Some(20000 + ((process::id() % 100) as u16) * 10);
    }

    loop {
        let port = next.expect("initialized above");
        *next = Some(if port >= 65000 {
            20000 + ((process::id() % 100) as u16) * 10
        } else {
            port + 1
        });

        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
}
