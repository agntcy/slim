use std::io::{BufRead, BufReader};
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Follows a child process stdout/stderr and accumulates output for repeated assertions.
pub struct ProcessLogWatcher {
    captured: Arc<Mutex<String>>,
}

impl ProcessLogWatcher {
    pub fn attach(child: &mut Child) -> Self {
        let captured = Arc::new(Mutex::new(String::new()));

        if let Some(stdout) = child.stdout.take() {
            let captured = Arc::clone(&captured);
            thread::spawn(move || {
                drain_stream(stdout, &captured);
            });
        }

        if let Some(stderr) = child.stderr.take() {
            let captured = Arc::clone(&captured);
            thread::spawn(move || {
                drain_stream(stderr, &captured);
            });
        }

        Self { captured }
    }

    pub fn wait_contains(&self, needle: &str, timeout: Duration) -> Result<(), String> {
        self.wait_matches(|text| text.contains(needle), timeout)
    }

    pub fn wait_matches<F>(&self, predicate: F, timeout: Duration) -> Result<(), String>
    where
        F: Fn(&str) -> bool,
    {
        let deadline = Instant::now() + timeout;

        loop {
            let captured = self
                .captured
                .lock()
                .expect("log capture mutex poisoned")
                .clone();
            if predicate(&captured) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(captured);
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn snapshot(&self) -> String {
        self.captured
            .lock()
            .expect("log capture mutex poisoned")
            .clone()
    }

    /// Assert `needle` does not appear in captured output for the full `duration`.
    pub fn wait_not_contains(&self, needle: &str, duration: Duration) -> Result<(), String> {
        let deadline = Instant::now() + duration;

        loop {
            let captured = self.snapshot();
            if captured.contains(needle) {
                return Err(format!(
                    "unexpected log line {needle:?} found in:\n{captured}"
                ));
            }
            if Instant::now() >= deadline {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(250));
        }
    }
}

fn drain_stream<R: std::io::Read>(stream: R, captured: &Arc<Mutex<String>>) {
    let reader = BufReader::new(stream);
    for line in reader.lines().map_while(Result::ok) {
        let mut buf = captured.lock().expect("log capture mutex poisoned");
        buf.push_str(&line);
        buf.push('\n');
    }
}

/// Poll merged stdout/stderr until `needle` appears or `timeout` elapses.
pub fn wait_for_log(child: &mut Child, needle: &str, timeout: Duration) -> Result<(), String> {
    let watcher = ProcessLogWatcher::attach(child);
    watcher.wait_contains(needle, timeout)
}

/// Returns true when `text` contains a semver-like fragment (e.g. `2.0.0`).
pub fn contains_semver_fragment(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 4 < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'.' {
            i += 1;
            continue;
        }
        j += 1;
        if j >= bytes.len() || !bytes[j].is_ascii_digit() {
            i += 1;
            continue;
        }
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'.' {
            i += 1;
            continue;
        }
        j += 1;
        if j >= bytes.len() || !bytes[j].is_ascii_digit() {
            i += 1;
            continue;
        }
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_fragment_detection() {
        assert!(contains_semver_fragment("remote_version=2.0.0-alpha.2"));
        assert!(!contains_semver_fragment("no version here"));
    }
}
