use std::path::Path;
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

const SLIMCTL_CM_RETRY_TIMEOUT: Duration = Duration::from_secs(30);

/// Run a command until it succeeds or `timeout` elapses, retrying every 200 ms.
///
/// Returns the successful [`Output`] so callers can layer `assert_cmd` assertions
/// (e.g. `output.assert().success().stdout(...)`) on top of the retry.
///
/// This retries on the *exit status* only. If the assertion that follows is
/// about output content that appears asynchronously — control-plane state, for
/// example — use [`run_combined_output_until`] instead: a command that exits 0
/// with nothing useful yet ends the retry here on the first attempt.
///
/// Each attempt calls `build_cmd()` so callers can construct a fresh `Command` (with args/env) per try.
pub fn run_combined_output_with_retry<F>(timeout: Duration, mut build_cmd: F) -> Output
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
                if output.status.success() {
                    return output;
                }
                last_out = combined_output(&output);
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

/// Run a command until it succeeds *and* its combined output satisfies
/// `is_ready`, or `timeout` elapses.
///
/// [`run_combined_output_with_retry`] returns on the first zero exit status,
/// which is not enough when the assertion is about asynchronous state rather
/// than about the command working: `slimctl controller route list` exits 0
/// while printing an empty table, so a caller that retries only on exit status
/// sees "0 route(s)" on the first attempt and never waits for the route to
/// propagate. Poll on the output instead.
///
/// Returns the combined output (stderr first) of the first attempt that
/// satisfies `is_ready`, or `Err` with the last output on timeout. Returning
/// rather than panicking lets the caller shut its test processes down before
/// failing.
pub fn run_combined_output_until<F, P>(
    timeout: Duration,
    mut build_cmd: F,
    mut is_ready: P,
) -> Result<Vec<u8>, String>
where
    F: FnMut() -> Command,
    P: FnMut(&str) -> bool,
{
    let deadline = Instant::now() + timeout;
    let mut last_cmd;
    let mut last_out;

    loop {
        let mut command = build_cmd();
        last_cmd = format!("{command:?}");

        match command.output() {
            Ok(output) => {
                last_out = combined_output(&output);
                if output.status.success() && is_ready(&String::from_utf8_lossy(&last_out)) {
                    return Ok(last_out);
                }
            }
            Err(err) => {
                last_out = format!("failed to execute: {err}").into_bytes();
            }
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "condition not met within {timeout:?}: {last_cmd}\nlast output:\n{}",
                String::from_utf8_lossy(&last_out)
            ));
        }
        thread::sleep(Duration::from_millis(200));
    }
}

/// Merge a command's stderr and stdout into a single buffer (stderr first).
pub(crate) fn combined_output(output: &Output) -> Vec<u8> {
    let mut combined = output.stderr.clone();
    combined.extend_from_slice(&output.stdout);
    combined
}

/// Run `slimctl cm …` until it succeeds or the channel-manager retry budget elapses.
///
/// Returns the successful [`Output`]; assert on it with `assert_cmd`, e.g.
/// `run_slimctl_cm(...).assert().success().stdout(predicate::str::contains("..."))`.
pub fn run_slimctl_cm(slimctl: &Path, cm_endpoint: &str, args: &[&str]) -> Output {
    let endpoint = cm_endpoint.to_string();
    run_combined_output_with_retry(SLIMCTL_CM_RETRY_TIMEOUT, || {
        let mut cmd = Command::new(slimctl);
        cmd.arg("cm");
        for arg in args {
            cmd.arg(arg);
        }
        cmd.arg("--server").arg(&endpoint);
        cmd
    })
}
