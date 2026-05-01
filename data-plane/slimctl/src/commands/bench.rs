// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! # slimctl bench
//!
//! Benchmark SLIM messaging performance against a running SLIM server.
//! Inspired by the design of `nats bench`.
//!
//! ## Usage
//!
//! ```text
//! # Terminal 1 — start subscribers first
//! slimctl bench sub --count 4 --msgs 1000000 --size 512
//!
//! # Terminal 2 — run publishers
//! slimctl bench pub --count 4 --msgs 1000000 --size 512
//!
//! # Request-reply latency measurement
//! slimctl bench sub --reply             # echo server
//! slimctl bench pub --request -n 10000  # latency client
//! ```
//!
//! Publishers and subscribers discover each other through predictable SLIM
//! names derived from `--prefix` and `--count`:
//!
//! ```text
//! sub-i identity: <prefix>/sub-{i}    (e.g. bench/test/sub-0)
//! pub-i identity: <prefix>/pub-{i}    (e.g. bench/test/pub-0)
//! ```
//!
//! The `--prefix` must match between `pub` and `sub` invocations.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use slim_bindings::{
    Name, SessionConfig, SessionType, get_global_service, initialize_with_defaults,
    new_insecure_client_config,
};

const DEFAULT_SERVER: &str = "http://localhost:46357";
const DEFAULT_SECRET: &str = "my_shared_secret_for_testing_purposes_only";
const DEFAULT_PREFIX: &str = "bench/test";

// ── Top-level args ─────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct BenchArgs {
    #[command(subcommand)]
    pub command: BenchCommand,
}

#[derive(Subcommand)]
pub enum BenchCommand {
    /// Run benchmark subscribers (start before publishers)
    Sub(BenchSubArgs),

    /// Run benchmark publishers
    Pub(BenchPubArgs),
}

// ── Sub args ───────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct BenchSubArgs {
    /// Number of concurrent subscriber apps
    #[arg(long, default_value_t = 1)]
    pub count: usize,

    /// Total messages to receive across all subscribers (0 = run until Ctrl+C)
    #[arg(short = 'n', long = "msgs", default_value_t = 100_000)]
    pub msgs: u64,

    /// Expected payload size in bytes (used for throughput calculation)
    #[arg(long = "size", default_value_t = 128)]
    pub size: usize,

    /// SLIM server URL
    #[arg(long, default_value = DEFAULT_SERVER)]
    pub server: String,

    /// Shared secret for authentication (min 32 chars)
    #[arg(long, default_value = DEFAULT_SECRET)]
    pub secret: String,

    /// Name prefix in org/namespace format (must match pub's prefix)
    #[arg(long, default_value = DEFAULT_PREFIX)]
    pub prefix: String,

    /// Reply mode: echo each received message back to the sender
    #[arg(long)]
    pub reply: bool,

    /// Append results to CSV file
    #[arg(long)]
    pub csv: Option<String>,

    /// Suppress per-second progress output
    #[arg(long)]
    pub no_progress: bool,
}

// ── Pub args ───────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct BenchPubArgs {
    /// Number of concurrent publisher apps
    #[arg(long, default_value_t = 1)]
    pub count: usize,

    /// Total messages to publish across all publishers
    #[arg(short = 'n', long = "msgs", default_value_t = 100_000)]
    pub msgs: u64,

    /// Payload size in bytes
    #[arg(long = "size", default_value_t = 128)]
    pub size: usize,

    /// SLIM server URL
    #[arg(long, default_value = DEFAULT_SERVER)]
    pub server: String,

    /// Shared secret for authentication (min 32 chars)
    #[arg(long, default_value = DEFAULT_SECRET)]
    pub secret: String,

    /// Name prefix in org/namespace format (must match sub's prefix)
    #[arg(long, default_value = DEFAULT_PREFIX)]
    pub prefix: String,

    /// Request-reply mode: wait for echo reply after each publish (measures latency)
    #[arg(long)]
    pub request: bool,

    /// Append results to CSV file
    #[arg(long)]
    pub csv: Option<String>,

    /// Suppress per-second progress output
    #[arg(long)]
    pub no_progress: bool,
}

// ── Dispatch ───────────────────────────────────────────────────────────────────

pub async fn run(args: &BenchArgs) -> Result<()> {
    match &args.command {
        BenchCommand::Sub(a) => run_sub(a).await,
        BenchCommand::Pub(a) => run_pub(a).await,
    }
}

// ── Internal measurement types ─────────────────────────────────────────────────

/// Metrics collected by a single publisher or subscriber worker.
struct Sample {
    /// Number of messages this worker was asked to process.
    job_msg_cnt: u64,
    /// Messages actually sent/received.
    msg_cnt: u64,
    /// Total payload bytes processed.
    msg_bytes: u64,
    start: Instant,
    end: Instant,
}

impl Sample {
    fn duration(&self) -> Duration {
        self.end.saturating_duration_since(self.start)
    }

    fn rate(&self) -> f64 {
        let s = self.duration().as_secs_f64();
        if s == 0.0 { 0.0 } else { self.job_msg_cnt as f64 / s }
    }

    fn throughput(&self) -> f64 {
        let s = self.duration().as_secs_f64();
        if s == 0.0 { 0.0 } else { self.msg_bytes as f64 / s }
    }
}

impl std::fmt::Display for Sample {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} msgs/sec ~ {}/sec",
            comma_format(self.rate() as i64),
            human_bytes(self.throughput()),
        )
    }
}

/// Aggregates samples from all workers using a time-window approach
/// (min start → max end across all samples), matching nats bench semantics.
struct SampleGroup {
    samples: Vec<Sample>,
    start: Option<Instant>,
    end: Option<Instant>,
    total_msgs: u64,
    total_bytes: u64,
    job_msg_cnt: u64,
}

impl SampleGroup {
    fn new() -> Self {
        Self {
            samples: Vec::new(),
            start: None,
            end: None,
            total_msgs: 0,
            total_bytes: 0,
            job_msg_cnt: 0,
        }
    }

    fn add(&mut self, s: Sample) {
        self.start = Some(match self.start {
            None => s.start,
            Some(existing) => existing.min(s.start),
        });
        self.end = Some(match self.end {
            None => s.end,
            Some(existing) => existing.max(s.end),
        });
        self.total_msgs += s.msg_cnt;
        self.total_bytes += s.msg_bytes;
        self.job_msg_cnt += s.job_msg_cnt;
        self.samples.push(s);
    }

    fn has_samples(&self) -> bool {
        !self.samples.is_empty()
    }

    fn window_duration(&self) -> Duration {
        match (self.start, self.end) {
            (Some(s), Some(e)) => e.saturating_duration_since(s),
            _ => Duration::ZERO,
        }
    }

    fn aggregate_rate(&self) -> f64 {
        let s = self.window_duration().as_secs_f64();
        if s == 0.0 { 0.0 } else { self.total_msgs as f64 / s }
    }

    fn aggregate_throughput(&self) -> f64 {
        let s = self.window_duration().as_secs_f64();
        if s == 0.0 { 0.0 } else { self.total_bytes as f64 / s }
    }

    fn min_rate(&self) -> f64 {
        self.samples.iter().map(|s| s.rate()).fold(f64::MAX, f64::min)
    }

    fn max_rate(&self) -> f64 {
        self.samples.iter().map(|s| s.rate()).fold(f64::MIN, f64::max)
    }

    fn avg_rate(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        self.samples.iter().map(|s| s.rate()).sum::<f64>() / self.samples.len() as f64
    }

    fn std_dev(&self) -> f64 {
        if self.samples.len() < 2 {
            return 0.0;
        }
        let avg = self.avg_rate();
        let variance = self.samples.iter()
            .map(|s| (s.rate() - avg).powi(2))
            .sum::<f64>()
            / self.samples.len() as f64;
        variance.sqrt()
    }

    fn statistics(&self) -> String {
        format!(
            "min {} | avg {} | max {} | stddev {} msgs/sec",
            comma_format(self.min_rate() as i64),
            comma_format(self.avg_rate() as i64),
            comma_format(self.max_rate() as i64),
            comma_format(self.std_dev() as i64),
        )
    }

    fn report(&self, label: &str) -> String {
        let mut out = format!(
            "{}: {} msgs/sec ~ {}/sec\n",
            label,
            comma_format(self.aggregate_rate() as i64),
            human_bytes(self.aggregate_throughput()),
        );
        if self.samples.len() > 1 {
            for (i, s) in self.samples.iter().enumerate() {
                out.push_str(&format!(
                    "  [{}] {} ({} msgs)\n",
                    i + 1,
                    s,
                    comma_format(s.job_msg_cnt as i64),
                ));
            }
            out.push_str(&format!("  {}\n", self.statistics()));
        }
        out
    }

    fn csv_rows(&self, run_id: &str, client_prefix: &str) -> String {
        self.samples.iter().enumerate().map(|(i, s)| {
            format!(
                "{},{}{},{},{},{:.0},{:.2}\n",
                run_id,
                client_prefix,
                i + 1,
                s.msg_cnt,
                s.msg_bytes,
                s.rate(),
                s.throughput(),
            )
        }).collect()
    }
}

/// Latency distribution collected in request-reply mode.
struct LatencyReport {
    samples: Vec<Duration>,
}

impl LatencyReport {
    fn new(mut samples: Vec<Duration>) -> Self {
        samples.sort_unstable();
        Self { samples }
    }

    fn percentile(&self, p: f64) -> Duration {
        if self.samples.is_empty() {
            return Duration::ZERO;
        }
        let idx = ((self.samples.len() - 1) as f64 * p / 100.0) as usize;
        self.samples[idx]
    }

    fn report(&self) -> String {
        if self.samples.is_empty() {
            return String::new();
        }
        format!(
            "\nLatency:\n  min    {}\n  p50    {}\n  p75    {}\n  p90    {}\n  p95    {}\n  p99    {}\n  max    {}\n",
            fmt_duration(*self.samples.first().unwrap()),
            fmt_duration(self.percentile(50.0)),
            fmt_duration(self.percentile(75.0)),
            fmt_duration(self.percentile(90.0)),
            fmt_duration(self.percentile(95.0)),
            fmt_duration(self.percentile(99.0)),
            fmt_duration(*self.samples.last().unwrap()),
        )
    }
}

// ── Name helpers ───────────────────────────────────────────────────────────────

/// Parse a `org/namespace` prefix string into its two components.
fn parse_prefix(prefix: &str) -> Result<(String, String)> {
    let mut parts = prefix.splitn(2, '/');
    let org = parts.next().unwrap_or("").trim();
    let ns = parts.next().unwrap_or("").trim();
    if org.is_empty() || ns.is_empty() {
        bail!("--prefix must be in org/namespace format, e.g. bench/test");
    }
    Ok((org.to_string(), ns.to_string()))
}

fn make_sub_name(org: &str, ns: &str, i: usize) -> Arc<Name> {
    Arc::new(Name::new(org.to_string(), ns.to_string(), format!("sub-{i}")))
}

fn make_pub_name(org: &str, ns: &str, i: usize) -> Arc<Name> {
    Arc::new(Name::new(org.to_string(), ns.to_string(), format!("pub-{i}")))
}

// ── Sub command ────────────────────────────────────────────────────────────────

async fn run_sub(args: &BenchSubArgs) -> Result<()> {
    if args.count == 0 {
        bail!("--count must be > 0");
    }

    let (org, ns) = parse_prefix(&args.prefix)?;

    // Distribute messages across workers; if msgs==0 each worker runs indefinitely.
    let msgs_per_worker = if args.msgs == 0 {
        0u64
    } else {
        let base = args.msgs / args.count as u64;
        // workers may get slightly different counts, we'll assign per-worker below
        base
    };
    let extra = if args.msgs == 0 { 0 } else { args.msgs % args.count as u64 };

    initialize_with_defaults();

    println!("SLIM Bench Sub: {} worker(s) on {}", args.count, args.server);
    println!(
        "  Listening at {}/{}/sub-0..sub-{}",
        org,
        ns,
        args.count - 1
    );
    if args.msgs > 0 {
        println!("  Expecting {} total messages", comma_format(args.msgs as i64));
    } else {
        println!("  Running until Ctrl+C");
    }
    if args.reply {
        println!("  Reply mode: ON (echoing messages back)");
    }
    println!("  Waiting for publishers...\n");

    let (result_tx, mut result_rx) = tokio::sync::mpsc::channel::<Sample>(args.count);

    let mut handles = Vec::with_capacity(args.count);
    for i in 0..args.count {
        let org = org.clone();
        let ns = ns.clone();
        let server = args.server.clone();
        let secret = args.secret.clone();
        let reply = args.reply;
        let size = args.size;
        let tx = result_tx.clone();
        // Give one extra message to the first `extra` workers.
        let job_cnt = msgs_per_worker + if (i as u64) < extra { 1 } else { 0 };

        let handle = tokio::spawn(async move {
            match run_sub_worker(i, &org, &ns, &server, &secret, job_cnt, size, reply).await {
                Ok(sample) => { let _ = tx.send(sample).await; }
                Err(e) => eprintln!("[sub-{i}] error: {e:#}"),
            }
        });
        handles.push(handle);
    }
    drop(result_tx);

    // Wait for all workers to finish.
    for h in handles {
        let _ = h.await;
    }

    // Aggregate and report.
    let mut group = SampleGroup::new();
    while let Some(sample) = result_rx.recv().await {
        group.add(sample);
    }

    if group.has_samples() {
        println!("\n{}", group.report("Sub stats"));

        if let Some(csv_path) = &args.csv {
            let run_id = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
            append_csv(csv_path, &group.csv_rows(&run_id, "S"))
                .with_context(|| format!("writing CSV to {csv_path}"))?;
            println!("Results written to {csv_path}");
        }
    } else {
        println!("No samples collected.");
    }

    Ok(())
}

async fn run_sub_worker(
    i: usize,
    org: &str,
    ns: &str,
    server: &str,
    secret: &str,
    job_msg_cnt: u64, // 0 = run until session closes or timeout
    size: usize,
    reply: bool,
) -> Result<Sample> {
    let own_name = make_sub_name(org, ns, i);

    let service = get_global_service();
    let conn_id = service
        .connect_async(new_insecure_client_config(server.to_string()))
        .await
        .context("connect to server failed")?;

    println!("[sub-{i}] connected (conn_id: {conn_id})");

    let app = service
        .create_app_with_secret_async(own_name.clone(), secret.to_string())
        .await
        .context("create app failed")?;

    app.subscribe_async(own_name.clone(), Some(conn_id))
        .await
        .context("subscribe failed")?;

    println!("[sub-{i}] ready at {own_name} — waiting for publisher session...");

    // Wait for a session from a publisher.  Poll with a 2-second timeout so
    // that if the channel returns any error we retry rather than giving up.
    // Any non-session notification (e.g. NewMessage arriving out of order) is
    // also retried: the notification is consumed from the channel and the loop
    // calls listen_for_session_async again, which will block until the real
    // NewSession notification arrives.
    let session = loop {
        match app
            .listen_for_session_async(Some(Duration::from_secs(2)))
            .await
        {
            Ok(s) => break s,
            Err(e) => {
                // Log non-timeout errors so the user can see unexpected issues,
                // but always retry — the session INIT may still be in transit.
                let msg = e.to_string().to_lowercase();
                if !msg.contains("timed out") {
                    eprintln!("[sub-{i}] listen_for_session: {e} (retrying)");
                }
                continue;
            }
        }
    };

    println!("[sub-{i}] session established");

    let recv_timeout = Duration::from_secs(30);
    let start = Instant::now();
    let mut msg_cnt = 0u64;
    let mut msg_bytes = 0u64;

    loop {
        match session.get_message_async(Some(recv_timeout)).await {
            Ok(msg) => {
                msg_bytes += msg.payload.len() as u64;
                msg_cnt += 1;

                if reply {
                    // Echo the payload back to the sender.
                    let payload = msg.payload.clone();
                    let _ = session
                        .publish_to_and_wait_async(msg.context, payload, None, None)
                        .await;
                }

                if job_msg_cnt > 0 && msg_cnt >= job_msg_cnt {
                    break;
                }
            }
            Err(_) => break, // timeout or session closed
        }
    }

    let end = Instant::now();

    // Fall back to the configured size hint if payload bytes were not captured
    // (e.g. empty payloads).
    if msg_bytes == 0 && msg_cnt > 0 {
        msg_bytes = msg_cnt * size as u64;
    }

    Ok(Sample {
        job_msg_cnt: if job_msg_cnt == 0 { msg_cnt } else { job_msg_cnt },
        msg_cnt,
        msg_bytes,
        start,
        end,
    })
}

// ── Pub command ────────────────────────────────────────────────────────────────

async fn run_pub(args: &BenchPubArgs) -> Result<()> {
    if args.count == 0 {
        bail!("--count must be > 0");
    }
    if args.msgs == 0 {
        bail!("--msgs must be > 0 for publishers");
    }

    let (org, ns) = parse_prefix(&args.prefix)?;

    let base = args.msgs / args.count as u64;
    let extra = args.msgs % args.count as u64;

    initialize_with_defaults();

    println!("SLIM Bench Pub: {} publisher(s) on {}", args.count, args.server);
    println!("  Total messages : {}", comma_format(args.msgs as i64));
    println!("  Payload size   : {} bytes", args.size);
    if args.request {
        println!("  Mode           : request-reply (measuring round-trip latency)");
    }
    println!();

    let payload = vec![0u8; args.size];

    let (result_tx, mut result_rx) =
        tokio::sync::mpsc::channel::<(Sample, Vec<Duration>)>(args.count);

    let mut handles = Vec::with_capacity(args.count);
    for i in 0..args.count {
        let org = org.clone();
        let ns = ns.clone();
        let server = args.server.clone();
        let secret = args.secret.clone();
        let request = args.request;
        let tx = result_tx.clone();
        let p = payload.clone();
        let msgs = base + if (i as u64) < extra { 1 } else { 0 };

        let handle = tokio::spawn(async move {
            match run_pub_worker(i, &org, &ns, &server, &secret, msgs, p, request).await {
                Ok((sample, latencies)) => { let _ = tx.send((sample, latencies)).await; }
                Err(e) => eprintln!("[pub-{i}] error: {e:#}"),
            }
        });
        handles.push(handle);
    }
    drop(result_tx);

    // Wait for all workers.
    for h in handles {
        let _ = h.await;
    }

    // Aggregate and report.
    let mut group = SampleGroup::new();
    let mut all_latencies = Vec::new();
    while let Some((sample, latencies)) = result_rx.recv().await {
        group.add(sample);
        all_latencies.extend(latencies);
    }

    if group.has_samples() {
        println!("\n{}", group.report("Pub stats"));

        if !all_latencies.is_empty() {
            let lr = LatencyReport::new(all_latencies);
            println!("{}", lr.report());
        }

        if let Some(csv_path) = &args.csv {
            let run_id = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
            append_csv(csv_path, &group.csv_rows(&run_id, "P"))
                .with_context(|| format!("writing CSV to {csv_path}"))?;
            println!("Results written to {csv_path}");
        }
    } else {
        println!("No samples collected.");
    }

    Ok(())
}

async fn run_pub_worker(
    i: usize,
    org: &str,
    ns: &str,
    server: &str,
    secret: &str,
    msg_count: u64,
    payload: Vec<u8>,
    request: bool,
) -> Result<(Sample, Vec<Duration>)> {
    let own_name = make_pub_name(org, ns, i);
    let target_name = make_sub_name(org, ns, i);

    let service = get_global_service();
    let conn_id = service
        .connect_async(new_insecure_client_config(server.to_string()))
        .await
        .context("connect to server failed")?;

    println!("[pub-{i}] connected (conn_id: {conn_id})");

    let app = service
        .create_app_with_secret_async(own_name.clone(), secret.to_string())
        .await
        .context("create app failed")?;

    // Subscribe own name so replies can reach us in request-reply mode.
    app.subscribe_async(own_name.clone(), Some(conn_id))
        .await
        .context("subscribe failed")?;

    // Advertise the route to the subscriber through the server connection.
    app.set_route_async(target_name.clone(), conn_id)
        .await
        .context("set route to subscriber failed")?;

    println!("[pub-{i}] subscribed {own_name} — creating session to {target_name}...");

    // Create a session to the subscriber.  Use generous retries so the
    // publisher can wait up to 30 seconds for the subscriber to come up.
    let session_config = SessionConfig {
        session_type: SessionType::PointToPoint,
        enable_mls: false,
        max_retries: Some(60),
        interval: Some(Duration::from_millis(500)),
        metadata: HashMap::new(),
    };

    // Use create_session_async + wait_for_async so the wall-clock timeout is
    // enforced independently of the session-layer retry count.  If the
    // session is not established within 35 s we return a clear error rather
    // than hanging forever.
    let swc = app
        .create_session_async(session_config, target_name.clone())
        .await
        .context("failed to initiate session")?;

    swc.completion
        .wait_for_async(Duration::from_secs(35))
        .await
        .with_context(|| {
            format!(
                "[pub-{i}] session to {target_name} not established within 35 s — \
                 is `slimctl bench sub` running with a matching --prefix and --count?"
            )
        })?;

    let session = swc.session;

    println!("[pub-{i}] session established — publishing {msg_count} messages");

    let start = Instant::now();
    let msg_bytes = msg_count * payload.len() as u64;
    let mut latencies = if request {
        Vec::with_capacity(msg_count as usize)
    } else {
        Vec::new()
    };

    let reply_timeout = Duration::from_secs(10);

    for _ in 0..msg_count {
        let t0 = Instant::now();

        session
            .publish_and_wait_async(payload.clone(), None, None)
            .await
            .context("publish failed")?;

        if request {
            session
                .get_message_async(Some(reply_timeout))
                .await
                .context("awaiting reply timed out")?;
            latencies.push(t0.elapsed());
        }
    }

    let end = Instant::now();

    Ok((
        Sample {
            job_msg_cnt: msg_count,
            msg_cnt: msg_count,
            msg_bytes,
            start,
            end,
        },
        latencies,
    ))
}

// ── Formatting utilities ───────────────────────────────────────────────────────

fn comma_format(n: i64) -> String {
    if n < 0 {
        return format!("-{}", comma_format(-n));
    }
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

fn human_bytes(bytes: f64) -> String {
    const BASE: f64 = 1024.0;
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if bytes < BASE {
        return format!("{bytes:.2} B");
    }
    let exp = (bytes.ln() / BASE.ln()) as usize;
    let idx = exp.min(UNITS.len() - 1);
    format!("{:.2} {}", bytes / BASE.powi(idx as i32), UNITS[idx])
}

fn fmt_duration(d: Duration) -> String {
    let micros = d.as_micros();
    if micros < 1_000 {
        format!("{micros}µs")
    } else if micros < 1_000_000 {
        format!("{:.2}ms", micros as f64 / 1_000.0)
    } else {
        format!("{:.2}s", d.as_secs_f64())
    }
}

fn append_csv(path: &str, rows: &str) -> Result<()> {
    use std::io::Write;

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open {path}"))?;

    // Write CSV header if the file is empty.
    if file.metadata()?.len() == 0 {
        writeln!(
            file,
            "RunID,ClientID,MsgCount,MsgBytes,MsgsPerSec,BytesPerSec"
        )?;
    }
    file.write_all(rows.as_bytes())?;
    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_prefix_valid() {
        let (org, ns) = parse_prefix("bench/test").unwrap();
        assert_eq!(org, "bench");
        assert_eq!(ns, "test");
    }

    #[test]
    fn parse_prefix_invalid_no_slash() {
        assert!(parse_prefix("benchtest").is_err());
    }

    #[test]
    fn parse_prefix_invalid_empty_parts() {
        assert!(parse_prefix("/test").is_err());
        assert!(parse_prefix("bench/").is_err());
    }

    #[test]
    fn make_names_correct() {
        let sub = make_sub_name("bench", "test", 3);
        let parts = sub.components();
        assert_eq!(parts, vec!["bench", "test", "sub-3"]);

        let pub_ = make_pub_name("bench", "test", 0);
        let parts = pub_.components();
        assert_eq!(parts, vec!["bench", "test", "pub-0"]);
    }

    #[test]
    fn comma_format_basic() {
        assert_eq!(comma_format(0), "0");
        assert_eq!(comma_format(999), "999");
        assert_eq!(comma_format(1_000), "1,000");
        assert_eq!(comma_format(1_234_567), "1,234,567");
        assert_eq!(comma_format(-1_000), "-1,000");
    }

    #[test]
    fn human_bytes_units() {
        assert!(human_bytes(500.0).contains("B"));
        assert!(human_bytes(1500.0).contains("KB"));
        assert!(human_bytes(2.0 * 1024.0 * 1024.0).contains("MB"));
    }

    #[test]
    fn fmt_duration_units() {
        assert_eq!(fmt_duration(Duration::from_micros(500)), "500µs");
        assert!(fmt_duration(Duration::from_millis(5)).contains("ms"));
        assert!(fmt_duration(Duration::from_secs(2)).contains("s"));
    }

    #[test]
    fn sample_rate() {
        let s = Sample {
            job_msg_cnt: 1_000,
            msg_cnt: 1_000,
            msg_bytes: 128_000,
            start: Instant::now(),
            end: Instant::now() + Duration::from_secs(1),
        };
        assert!((s.rate() - 1_000.0).abs() < 1.0);
        assert!((s.throughput() - 128_000.0).abs() < 1.0);
    }

    #[test]
    fn sample_group_aggregate() {
        let now = Instant::now();
        let mut g = SampleGroup::new();
        g.add(Sample {
            job_msg_cnt: 500,
            msg_cnt: 500,
            msg_bytes: 64_000,
            start: now,
            end: now + Duration::from_secs(1),
        });
        g.add(Sample {
            job_msg_cnt: 500,
            msg_cnt: 500,
            msg_bytes: 64_000,
            start: now,
            end: now + Duration::from_secs(1),
        });
        assert!(g.has_samples());
        // Aggregate over the same 1s window → 1000 msgs/s
        assert!((g.aggregate_rate() - 1_000.0).abs() < 1.0);
    }

    #[test]
    fn latency_report_percentiles() {
        let samples: Vec<Duration> = (1..=100).map(|i| Duration::from_micros(i)).collect();
        let lr = LatencyReport::new(samples);
        assert_eq!(lr.percentile(50.0), Duration::from_micros(50));
        assert_eq!(lr.percentile(99.0), Duration::from_micros(99));
        assert!(!lr.report().is_empty());
    }
}
