use std::process::{self, Child};
use std::thread;
use std::time::{Duration, Instant};

/// Gracefully stop a spawned subprocess, then force-kill if it does not exit in time.
pub fn terminate_session(session: &mut Option<Child>, timeout: Duration) {
    let Some(mut child) = session.take() else {
        return;
    };

    send_terminate_signal(&child);
    if !wait_for_exit(&mut child, timeout) {
        let _ = child.kill();
        let _ = wait_for_exit(&mut child, Duration::from_secs(5));
    }
}

/// Assert a child process keeps running for the full `duration`.
pub fn session_still_running(child: &mut Child, duration: Duration) -> Result<(), String> {
    let deadline = Instant::now() + duration;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(format!("process exited early with status: {status}"));
            }
            Ok(None) if Instant::now() >= deadline => return Ok(()),
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(err) => return Err(format!("try_wait failed: {err}")),
        }
    }
}

fn send_terminate_signal(child: &Child) {
    #[cfg(unix)]
    {
        let pid = child.id().to_string();
        let _ = process::Command::new("kill")
            .arg("-TERM")
            .arg(&pid)
            .status();
    }

    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;

    loop {
        match child.try_wait() {
            Ok(Some(_status)) => return true,
            Ok(None) if Instant::now() >= deadline => return false,
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => return true,
        }
    }
}
