use super::command::run_combined_output_with_retry;
use regex::Regex;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

/// Poll `slimctl n route list` until a route matching `prefix/<uuid>` appears.
pub fn wait_for_route_with_uuid_suffix(
    slimctl: &Path,
    controller_endpoint: &str,
    route_prefix: &str,
    timeout: Duration,
) -> String {
    let pattern = format!(r"({}/[0-9a-f-]+)", regex::escape(route_prefix));
    let re = Regex::new(&pattern).expect("valid route uuid regex");
    wait_for_route_match(slimctl, controller_endpoint, &re, route_prefix, timeout)
}

/// Poll `slimctl n route list` until a route matching `prefix/<id> connections=` appears.
pub fn wait_for_route_with_connections_format(
    slimctl: &Path,
    controller_endpoint: &str,
    route_prefix: &str,
    timeout: Duration,
) -> String {
    let pattern = format!(r"({}/\S+)\s+connections=", regex::escape(route_prefix));
    let re = Regex::new(&pattern).expect("valid route connections regex");
    wait_for_route_match(slimctl, controller_endpoint, &re, route_prefix, timeout)
}

fn wait_for_route_match(
    slimctl: &Path,
    controller_endpoint: &str,
    re: &Regex,
    route_prefix: &str,
    timeout: Duration,
) -> String {
    let deadline = Instant::now() + timeout;
    let mut last_output = run_slimctl_node_output(slimctl, controller_endpoint, &["route", "list"]);

    loop {
        if let Some(captures) = re.captures(&last_output)
            && let Some(route) = captures.get(1)
        {
            return route.as_str().to_string();
        }

        if Instant::now() >= deadline {
            panic!(
                "timed out waiting for route with real app ID for {route_prefix}:\n{last_output}"
            );
        }
        thread::sleep(Duration::from_millis(500));
        last_output = run_slimctl_node_output(slimctl, controller_endpoint, &["route", "list"]);
    }
}

pub fn run_slimctl_node_output(slimctl: &Path, controller_endpoint: &str, args: &[&str]) -> String {
    let output = run_slimctl_node(slimctl, controller_endpoint, args)
        .unwrap_or_else(|err| panic!("slimctl n {} failed: {err}", args.join(" ")));
    String::from_utf8_lossy(&output).into_owned()
}

pub fn run_slimctl_node_retry(
    slimctl: &Path,
    controller_endpoint: &str,
    args: &[&str],
    timeout: Duration,
) -> Vec<u8> {
    let endpoint = controller_endpoint.to_string();
    run_combined_output_with_retry(timeout, || {
        let mut cmd = Command::new(slimctl);
        cmd.arg("n");
        for arg in args {
            cmd.arg(arg);
        }
        cmd.arg("--server").arg(&endpoint);
        cmd
    })
}

pub fn run_slimctl_node_add_route_via(
    slimctl: &Path,
    controller_endpoint: &str,
    route: &str,
    via_config: &Path,
    timeout: Duration,
) -> Vec<u8> {
    run_slimctl_node_retry(
        slimctl,
        controller_endpoint,
        &[
            "route",
            "add",
            route,
            "via",
            via_config.to_str().expect("utf-8 via path"),
        ],
        timeout,
    )
}

pub fn run_slimctl_controller_retry(
    slimctl: &Path,
    controller_endpoint: &str,
    args: &[&str],
    timeout: Duration,
) -> Vec<u8> {
    let endpoint = controller_endpoint.to_string();
    run_combined_output_with_retry(timeout, || {
        let mut cmd = Command::new(slimctl);
        cmd.arg("controller");
        for arg in args {
            cmd.arg(arg);
        }
        cmd.arg("--server").arg(&endpoint);
        cmd
    })
}

fn run_slimctl_node(
    slimctl: &Path,
    controller_endpoint: &str,
    args: &[&str],
) -> Result<Vec<u8>, String> {
    let mut cmd = Command::new(slimctl);
    cmd.arg("n");
    for arg in args {
        cmd.arg(arg);
    }
    cmd.arg("--server").arg(controller_endpoint);

    let output = cmd
        .output()
        .map_err(|err| format!("failed to execute slimctl: {err}"))?;
    if !output.status.success() {
        let mut combined = output.stderr;
        combined.extend_from_slice(&output.stdout);
        return Err(String::from_utf8_lossy(&combined).into_owned());
    }

    let mut combined = output.stdout;
    combined.extend_from_slice(&output.stderr);
    Ok(combined)
}
