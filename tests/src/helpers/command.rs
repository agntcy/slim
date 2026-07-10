use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

/// Run a command until it succeeds or `timeout` elapses, retrying every 200 ms.
///
/// Each attempt calls `build_cmd()` so callers can construct a fresh `Command` (with args/env) per try.
pub fn run_combined_output_with_retry<F>(timeout: Duration, mut build_cmd: F) -> Vec<u8>
where
    F: FnMut() -> Command,
{
    let deadline = Instant::now() + timeout;
    let mut last_out = Vec::new();
    let mut last_err;
    let mut last_cmd;

    loop {
        let mut command = build_cmd();
        last_cmd = format!("{command:?}");

        match command.output() {
            Ok(output) => {
                last_out = combined_output(&output);
                if output.status.success() {
                    return last_out;
                }
                last_err = std::io::Error::other(format!("process exited with {}", output.status));
            }
            Err(err) => {
                last_err = err;
            }
        }

        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }

    panic!(
        "command failed after retry: {last_cmd}\nerror: {last_err}\noutput:\n{}",
        String::from_utf8_lossy(&last_out)
    );
}

fn combined_output(output: &Output) -> Vec<u8> {
    let mut combined = output.stderr.clone();
    combined.extend_from_slice(&output.stdout);
    combined
}
