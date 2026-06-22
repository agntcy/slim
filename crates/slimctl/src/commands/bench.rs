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

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::traits::{TokenProvider, Verifier};
use slim_config::client::ClientConfig;
use slim_config::component::ComponentBuilder;
use slim_config::tls::client::TlsClientConfig;
use slim_datapath::api::{ProtoName, ProtoPublishType, ProtoSessionType};
use slim_service::app::App;
use slim_service::{Service, ServiceBuilder};
use slim_session::context::SessionContext;
use slim_session::{AppChannelReceiver, Direction, Notification, SessionConfig, SessionError};
use slim_tracing::TracingConfiguration;
use tokio::task::JoinSet;

const DEFAULT_SERVER: &str = "http://localhost:46357";
const DEFAULT_SECRET: &str = "my_shared_secret_for_testing_purposes_only";
const DEFAULT_PREFIX: &str = "bench/test";

/// Timeout waiting to receive a message inside a benchmarked session.
const RECV_TIMEOUT: Duration = Duration::from_secs(30);
/// Wall-clock budget for a publisher/inviter to establish a session.
const SESSION_ESTABLISH_TIMEOUT: Duration = Duration::from_secs(35);
/// Maximum session-layer retries while waiting for the peer.
const SESSION_MAX_RETRIES: u32 = 60;
/// Interval between session-layer retries.
const SESSION_RETRY_INTERVAL: Duration = Duration::from_millis(500);

// ── Top-level args ─────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct BenchArgs {
    /// Log level for SLIM internals ("error", "warn", "info", "debug", "trace")
    #[arg(long, default_value = "info", global = true)]
    pub log_level: String,

    #[command(subcommand)]
    pub command: BenchCommand,
}

#[derive(Subcommand)]
pub enum BenchCommand {
    /// Run benchmark subscribers (start before publishers)
    Sub(BenchSubArgs),

    /// Run benchmark publishers
    Pub(BenchPubArgs),

    /// Run channel (group session) benchmark — one publisher broadcasts to N subscribers
    Channel(BenchChannelArgs),
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

    /// Payload size — plain bytes or a unit suffix. SI: kb/mb/gb (×1000). IEC: kib/mib/gib (×1024).
    #[arg(long = "size", default_value = "128", value_parser = parse_byte_size)]
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

    /// Payload size — plain bytes or a unit suffix. SI: kb/mb/gb (×1000). IEC: kib/mib/gib (×1024).
    #[arg(long = "size", default_value = "128", value_parser = parse_byte_size)]
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
}

// ── Channel args ───────────────────────────────────────────────────────────────

/// Top-level args for `bench channel`.
#[derive(Args)]
pub struct BenchChannelArgs {
    #[command(subcommand)]
    pub command: BenchChannelCommand,
}

#[derive(Subcommand)]
pub enum BenchChannelCommand {
    /// Start subscriber workers that join a channel (run before `bench channel pub`)
    Sub(BenchChannelSubArgs),
    /// Run the channel publisher: creates a group session, invites all subscribers, broadcasts
    Pub(BenchChannelPubArgs),
}

#[derive(Args)]
pub struct BenchChannelSubArgs {
    /// Number of subscriber apps to join the channel (must match pub's --count)
    #[arg(long, default_value_t = 2)]
    pub count: usize,

    /// Messages to receive per subscriber before reporting (0 = run until Ctrl+C)
    #[arg(short = 'n', long = "msgs", default_value_t = 100_000)]
    pub msgs: u64,

    /// Payload size — plain bytes or a unit suffix. SI: kb/mb/gb (×1000). IEC: kib/mib/gib (×1024).
    #[arg(long = "size", default_value = "128", value_parser = parse_byte_size)]
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

    /// Append results to CSV file
    #[arg(long)]
    pub csv: Option<String>,

    /// Index of the first subscriber in this process.
    /// Use with --count to distribute subscribers across multiple processes.
    /// Example: process A: --count 2 --start-index 0  (registers channel-sub-0, channel-sub-1)
    ///          process B: --count 2 --start-index 2  (registers channel-sub-2, channel-sub-3)
    #[arg(long, default_value_t = 0)]
    pub start_index: usize,
}

#[derive(Args)]
pub struct BenchChannelPubArgs {
    /// Number of subscribers to invite into the channel (must match `bench channel sub --count`)
    #[arg(long, default_value_t = 2)]
    pub count: usize,

    /// Total messages to broadcast to the channel
    #[arg(short = 'n', long = "msgs", default_value_t = 100_000)]
    pub msgs: u64,

    /// Payload size — plain bytes or a unit suffix. SI: kb/mb/gb (×1000). IEC: kib/mib/gib (×1024).
    #[arg(long = "size", default_value = "128", value_parser = parse_byte_size)]
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

    /// Append results to CSV file
    #[arg(long)]
    pub csv: Option<String>,
}

// ── Dispatch ───────────────────────────────────────────────────────────────────

pub async fn run(args: &BenchArgs) -> Result<()> {
    let tracing = TracingConfiguration::default().with_log_level(args.log_level.clone());
    let _guard = tracing
        .setup_tracing_subscriber()
        .context("failed to setup tracing")?;

    slim_config::tls::provider::initialize_crypto_provider();

    let service = Arc::new(
        ServiceBuilder::new()
            .build("bench-service".to_string())
            .context("failed to create SLIM service")?,
    );

    match &args.command {
        BenchCommand::Sub(a) => run_sub(a, &service).await,
        BenchCommand::Pub(a) => run_pub(a, &service).await,
        BenchCommand::Channel(a) => run_channel(a, &service).await,
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
        if s == 0.0 {
            0.0
        } else {
            self.job_msg_cnt as f64 / s
        }
    }

    fn throughput(&self) -> f64 {
        let s = self.duration().as_secs_f64();
        if s == 0.0 {
            0.0
        } else {
            self.msg_bytes as f64 / s
        }
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

/// Build a [`Sample`] from the raw counters collected inside a receive loop.
///
/// `start` is `None` when no messages arrived (the first message sets it).
/// Falls back to `size` as a byte-count hint when `msg_bytes` is zero
/// (e.g. the receive path doesn't capture payload length).
fn build_sample(
    start: Option<Instant>,
    msg_cnt: u64,
    mut msg_bytes: u64,
    job_msg_cnt: u64,
    size: usize,
) -> Sample {
    let end = Instant::now();
    let start = start.unwrap_or(end);
    if msg_bytes == 0 && msg_cnt > 0 {
        msg_bytes = msg_cnt * size as u64;
    }
    Sample {
        job_msg_cnt: if job_msg_cnt == 0 {
            msg_cnt
        } else {
            job_msg_cnt
        },
        msg_cnt,
        msg_bytes,
        start,
        end,
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
        if s == 0.0 {
            0.0
        } else {
            self.total_msgs as f64 / s
        }
    }

    fn aggregate_throughput(&self) -> f64 {
        let s = self.window_duration().as_secs_f64();
        if s == 0.0 {
            0.0
        } else {
            self.total_bytes as f64 / s
        }
    }

    fn min_rate(&self) -> f64 {
        self.samples
            .iter()
            .map(|s| s.rate())
            .fold(f64::MAX, f64::min)
    }

    fn max_rate(&self) -> f64 {
        self.samples
            .iter()
            .map(|s| s.rate())
            .fold(f64::MIN, f64::max)
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
        let variance = self
            .samples
            .iter()
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
        self.samples
            .iter()
            .enumerate()
            .map(|(i, s)| {
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
            })
            .collect()
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

fn make_sub_name(org: &str, ns: &str, i: usize) -> ProtoName {
    ProtoName::from_strings([org, ns, &format!("sub-{i}")])
}

fn make_pub_name(org: &str, ns: &str, i: usize) -> ProtoName {
    ProtoName::from_strings([org, ns, &format!("pub-{i}")])
}

fn make_channel_name(org: &str, ns: &str) -> ProtoName {
    ProtoName::from_strings([org, ns, "channel"])
}

fn make_channel_pub_name(org: &str, ns: &str) -> ProtoName {
    ProtoName::from_strings([org, ns, "channel-pub"])
}

fn make_channel_sub_name(org: &str, ns: &str, i: usize) -> ProtoName {
    ProtoName::from_strings([org, ns, &format!("channel-sub-{i}")])
}

async fn create_app_with_secret(
    service: &Service,
    name: &ProtoName,
    secret: &str,
) -> Result<(
    App<AuthProvider, AuthVerifier>,
    tokio::sync::mpsc::Receiver<Result<Notification, SessionError>>,
)> {
    let name_str = name.to_string();
    let mut provider = AuthProvider::shared_secret_from_str(&name_str, secret)
        .context("failed to create auth provider")?;
    let mut verifier = AuthVerifier::shared_secret_from_str(&name_str, secret)
        .context("failed to create auth verifier")?;
    provider
        .initialize()
        .await
        .map_err(|e| anyhow!("provider init failed: {e}"))?;
    verifier
        .initialize()
        .await
        .map_err(|e| anyhow!("verifier init failed: {e}"))?;
    let (app, rx) = service
        .create_app_with_direction(name, provider, verifier, Direction::Bidirectional)
        .context("failed to create app")?;
    Ok((app, rx))
}

fn extract_payload(msg: &slim_datapath::api::ProtoMessage) -> Vec<u8> {
    match msg.message_type.as_ref() {
        Some(ProtoPublishType(publish)) => publish
            .msg
            .as_ref()
            .and_then(|content| content.as_application_payload().ok())
            .map(|payload| payload.blob.clone())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

async fn recv_session_message(
    session_rx: &mut AppChannelReceiver,
    recv_timeout: Duration,
) -> Option<slim_datapath::api::ProtoMessage> {
    match tokio::time::timeout(recv_timeout, session_rx.recv()).await {
        Ok(Some(Ok(msg))) => Some(msg),
        _ => None,
    }
}

/// Wait for an incoming session notification, retrying on timeouts and non-session events.
async fn wait_for_incoming_session(
    notification_rx: &mut tokio::sync::mpsc::Receiver<Result<Notification, SessionError>>,
    label: &str,
) -> Result<SessionContext> {
    loop {
        match tokio::time::timeout(Duration::from_secs(2), notification_rx.recv()).await {
            Ok(Some(Ok(Notification::NewSession(ctx)))) => return Ok(ctx),
            Ok(Some(Ok(Notification::NewMessage(_)))) => continue,
            Ok(Some(Err(e))) => {
                eprintln!("[{label}] notification error: {e} (retrying)");
                continue;
            }
            Ok(None) => {
                bail!("notification channel closed");
            }
            Err(_) => continue,
        }
    }
}

/// Create an outgoing session, wait for it to be established, and return the context.
async fn create_and_wait_session(
    app: &App<AuthProvider, AuthVerifier>,
    session_type: ProtoSessionType,
    destination: ProtoName,
    timeout: Duration,
    timeout_msg: &str,
) -> Result<SessionContext> {
    let session_config = SessionConfig {
        session_type,
        mls_settings: None,
        max_retries: Some(SESSION_MAX_RETRIES),
        interval: Some(SESSION_RETRY_INTERVAL),
        initiator: true,
        metadata: HashMap::new(),
    };

    let (session_ctx, completion) = app
        .create_session(session_config, destination, None)
        .await
        .context("failed to create session")?;

    tokio::time::timeout(timeout, completion)
        .await
        .context(timeout_msg.to_string())?
        .context("session establishment failed")?;

    Ok(session_ctx)
}

// ── Sub command ────────────────────────────────────────────────────────────────

async fn run_sub(args: &BenchSubArgs, service: &Arc<Service>) -> Result<()> {
    if args.count == 0 {
        bail!("--count must be > 0");
    }

    let (org, ns) = parse_prefix(&args.prefix)?;

    // Distribute messages across workers; if msgs==0 each worker runs indefinitely.
    let msgs_per_worker = if args.msgs == 0 {
        0u64
    } else {
        // workers may get slightly different counts, we'll assign per-worker below
        args.msgs / args.count as u64
    };
    let extra = if args.msgs == 0 {
        0
    } else {
        args.msgs % args.count as u64
    };

    println!(
        "SLIM Bench Sub: {} worker(s) on {}",
        args.count, args.server
    );
    println!(
        "  Listening at {}/{}/sub-0..sub-{}",
        org,
        ns,
        args.count - 1
    );
    if args.msgs > 0 {
        println!(
            "  Expecting {} total messages",
            comma_format(args.msgs as i64)
        );
    } else {
        println!("  Running until Ctrl+C");
    }
    if args.reply {
        println!("  Reply mode: ON (echoing messages back)");
    }
    println!("  Waiting for publishers...\n");

    let client_config =
        ClientConfig::with_endpoint(&args.server).with_tls_setting(TlsClientConfig::insecure());
    let conn_id = service
        .connect(&client_config)
        .await
        .context("connect to server failed")?;

    let mut join_set: JoinSet<(usize, Result<Sample>)> = JoinSet::new();
    for i in 0..args.count {
        let org = org.clone();
        let ns = ns.clone();
        let secret = args.secret.clone();
        let reply = args.reply;
        let size = args.size;
        let service = service.clone();
        // Give one extra message to the first `extra` workers.
        let job_cnt = msgs_per_worker + if (i as u64) < extra { 1 } else { 0 };

        join_set.spawn(async move {
            (
                i,
                run_sub_worker(
                    &service, i, &org, &ns, &secret, conn_id, job_cnt, size, reply,
                )
                .await,
            )
        });
    }

    // Aggregate and report — processed in completion order.
    let mut group = SampleGroup::new();
    while let Some(res) = join_set.join_next().await {
        match res {
            Ok((_i, Ok(sample))) => group.add(sample),
            Ok((i, Err(e))) => eprintln!("[sub-{i}] error: {e:#}"),
            Err(e) => eprintln!("sub worker panicked: {e}"),
        }
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

#[allow(clippy::too_many_arguments)]
async fn run_sub_worker(
    service: &Service,
    i: usize,
    org: &str,
    ns: &str,
    secret: &str,
    conn_id: u64,
    job_msg_cnt: u64, // 0 = run until session closes or timeout
    size: usize,
    reply: bool,
) -> Result<Sample> {
    let own_name = make_sub_name(org, ns, i);

    let (app, mut notification_rx) = create_app_with_secret(service, &own_name, secret).await?;

    app.subscribe(&own_name, Some(conn_id))
        .await
        .context("subscribe failed")?;

    println!("[sub-{i}] ready at {own_name} — waiting for publisher session...");

    let session_ctx = wait_for_incoming_session(&mut notification_rx, &format!("sub-{i}")).await?;

    let controller = session_ctx
        .session_arc()
        .ok_or_else(|| anyhow!("session closed"))?;
    let mut session_rx = session_ctx.rx;

    println!("[sub-{i}] session established");

    let recv_timeout = RECV_TIMEOUT;
    let mut start: Option<Instant> = None;
    let mut msg_cnt = 0u64;
    let mut msg_bytes = 0u64;

    while let Some(msg) = recv_session_message(&mut session_rx, recv_timeout).await {
        if start.is_none() {
            start = Some(Instant::now());
        }
        let payload = extract_payload(&msg);
        msg_bytes += payload.len() as u64;
        msg_cnt += 1;

        if reply {
            let source = msg.get_source();
            let input_conn = msg.get_incoming_conn();
            let _ = controller
                .publish_to(&source, input_conn, payload, None, None)
                .await?
                .await;
        }

        if job_msg_cnt > 0 && msg_cnt >= job_msg_cnt {
            break;
        }
    }

    Ok(build_sample(start, msg_cnt, msg_bytes, job_msg_cnt, size))
}

// ── Pub command ────────────────────────────────────────────────────────────────

async fn run_pub(args: &BenchPubArgs, service: &Arc<Service>) -> Result<()> {
    if args.count == 0 {
        bail!("--count must be > 0");
    }
    if args.msgs == 0 {
        bail!("--msgs must be > 0 for publishers");
    }

    let (org, ns) = parse_prefix(&args.prefix)?;

    let base = args.msgs / args.count as u64;
    let extra = args.msgs % args.count as u64;

    println!(
        "SLIM Bench Pub: {} publisher(s) on {}",
        args.count, args.server
    );
    println!("  Total messages : {}", comma_format(args.msgs as i64));
    println!("  Payload size   : {} bytes", args.size);
    if args.request {
        println!("  Mode           : request-reply (measuring round-trip latency)");
    }
    println!();

    let payload = vec![0u8; args.size];

    let client_config =
        ClientConfig::with_endpoint(&args.server).with_tls_setting(TlsClientConfig::insecure());
    let conn_id = service
        .connect(&client_config)
        .await
        .context("connect to server failed")?;

    let mut join_set: JoinSet<(usize, Result<(Sample, Vec<Duration>)>)> = JoinSet::new();
    for i in 0..args.count {
        let org = org.clone();
        let ns = ns.clone();
        let secret = args.secret.clone();
        let request = args.request;
        let p = payload.clone();
        let msgs = base + if (i as u64) < extra { 1 } else { 0 };
        let service = service.clone();

        join_set.spawn(async move {
            (
                i,
                run_pub_worker(&service, i, &org, &ns, &secret, conn_id, msgs, p, request).await,
            )
        });
    }

    // Aggregate and report — processed in completion order.
    let mut group = SampleGroup::new();
    let mut all_latencies = Vec::new();
    while let Some(res) = join_set.join_next().await {
        match res {
            Ok((_i, Ok((sample, latencies)))) => {
                group.add(sample);
                all_latencies.extend(latencies);
            }
            Ok((i, Err(e))) => eprintln!("[pub-{i}] error: {e:#}"),
            Err(e) => eprintln!("pub worker panicked: {e}"),
        }
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

#[allow(clippy::too_many_arguments)]
async fn run_pub_worker(
    service: &Service,
    i: usize,
    org: &str,
    ns: &str,
    secret: &str,
    conn_id: u64,
    msg_count: u64,
    payload: Vec<u8>,
    request: bool,
) -> Result<(Sample, Vec<Duration>)> {
    let own_name = make_pub_name(org, ns, i);
    let target_name = make_sub_name(org, ns, i);

    let (app, _notification_rx) = create_app_with_secret(service, &own_name, secret).await?;

    app.subscribe(&own_name, Some(conn_id))
        .await
        .context("subscribe failed")?;

    app.set_route(&target_name, conn_id)
        .await
        .context("set route to subscriber failed")?;

    println!("[pub-{i}] subscribed {own_name} — creating session to {target_name}...");

    let session_ctx = create_and_wait_session(
        &app,
        ProtoSessionType::PointToPoint,
        target_name.clone(),
        SESSION_ESTABLISH_TIMEOUT,
        &format!(
            "[pub-{i}] session to {target_name} not established within 35 s — \
             is `slimctl bench sub` running with a matching --prefix and --count?"
        ),
    )
    .await?;

    let (weak_session, mut session_rx) = session_ctx.into_parts();
    let controller = weak_session
        .upgrade()
        .ok_or_else(|| anyhow!("session closed"))?;

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

        controller
            .publish(controller.dst(), payload.clone(), None, None)
            .await
            .context("publish failed")?
            .await
            .context("publish completion failed")?;

        if request {
            recv_session_message(&mut session_rx, reply_timeout)
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

// ── Channel command ────────────────────────────────────────────────────────────

async fn run_channel(args: &BenchChannelArgs, service: &Arc<Service>) -> Result<()> {
    match &args.command {
        BenchChannelCommand::Sub(a) => run_channel_sub(a, service).await,
        BenchChannelCommand::Pub(a) => run_channel_pub(a, service).await,
    }
}

async fn run_channel_sub(args: &BenchChannelSubArgs, service: &Arc<Service>) -> Result<()> {
    if args.count == 0 {
        bail!("--count must be > 0");
    }

    let (org, ns) = parse_prefix(&args.prefix)?;

    println!(
        "SLIM Bench Channel Sub: {} worker(s) on {}",
        args.count, args.server
    );
    println!(
        "  Listening at {}/{}/channel-sub-{}..channel-sub-{}",
        org,
        ns,
        args.start_index,
        args.start_index + args.count - 1
    );
    if args.msgs > 0 {
        println!(
            "  Expecting {} messages per subscriber",
            comma_format(args.msgs as i64)
        );
    } else {
        println!("  Running until Ctrl+C");
    }
    println!("  Waiting for publisher to invite...\n");

    let client_config =
        ClientConfig::with_endpoint(&args.server).with_tls_setting(TlsClientConfig::insecure());
    let conn_id = service
        .connect(&client_config)
        .await
        .context("connect to server failed")?;

    let start_index = args.start_index;
    let mut join_set: JoinSet<(usize, Result<Sample>)> = JoinSet::new();
    for i in 0..args.count {
        let actual_index = start_index + i;
        let org = org.clone();
        let ns = ns.clone();
        let secret = args.secret.clone();
        let size = args.size;
        let msgs = args.msgs;
        let service = service.clone();
        join_set.spawn(async move {
            (
                actual_index,
                run_channel_sub_worker(
                    &service,
                    actual_index,
                    &org,
                    &ns,
                    &secret,
                    conn_id,
                    msgs,
                    size,
                )
                .await,
            )
        });
    }

    // Aggregate and report — processed in completion order.
    let mut group = SampleGroup::new();
    while let Some(res) = join_set.join_next().await {
        match res {
            Ok((_idx, Ok(sample))) => group.add(sample),
            Ok((idx, Err(e))) => eprintln!("[ch-sub-{idx}] error: {e:#}"),
            Err(e) => eprintln!("channel sub worker panicked: {e}"),
        }
    }

    if group.has_samples() {
        println!("\n{}", group.report("Channel sub stats"));
        if let Some(csv_path) = &args.csv {
            let run_id = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
            append_csv(csv_path, &group.csv_rows(&run_id, "CS"))
                .with_context(|| format!("writing CSV to {csv_path}"))?;
            println!("Results written to {csv_path}");
        }
    } else {
        println!("No samples collected.");
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_channel_sub_worker(
    service: &Service,
    i: usize,
    org: &str,
    ns: &str,
    secret: &str,
    conn_id: u64,
    job_msg_cnt: u64,
    size: usize,
) -> Result<Sample> {
    let own_name = make_channel_sub_name(org, ns, i);

    let (app, mut notification_rx) = create_app_with_secret(service, &own_name, secret).await?;

    app.subscribe(&own_name, Some(conn_id))
        .await
        .context("subscribe failed")?;

    println!("[ch-sub-{i}] ready at {own_name} — waiting for channel invite...");

    let session_ctx =
        wait_for_incoming_session(&mut notification_rx, &format!("ch-sub-{i}")).await?;

    let _controller = session_ctx
        .session_arc()
        .ok_or_else(|| anyhow!("session closed"))?;
    let mut session_rx = session_ctx.rx;

    println!("[ch-sub-{i}] joined channel session");

    let recv_timeout = RECV_TIMEOUT;
    let mut start: Option<Instant> = None;
    let mut msg_cnt = 0u64;
    let mut msg_bytes = 0u64;

    while let Some(msg) = recv_session_message(&mut session_rx, recv_timeout).await {
        if start.is_none() {
            start = Some(Instant::now());
        }
        msg_bytes += extract_payload(&msg).len() as u64;
        msg_cnt += 1;
        if job_msg_cnt > 0 && msg_cnt >= job_msg_cnt {
            break;
        }
    }

    Ok(build_sample(start, msg_cnt, msg_bytes, job_msg_cnt, size))
}

async fn run_channel_pub(args: &BenchChannelPubArgs, service: &Arc<Service>) -> Result<()> {
    if args.count == 0 {
        bail!("--count must be > 0");
    }
    if args.msgs == 0 {
        bail!("--msgs must be > 0 for publishers");
    }

    let (org, ns) = parse_prefix(&args.prefix)?;

    println!(
        "SLIM Bench Channel Pub: 1 publisher → {} subscriber(s) on {}",
        args.count, args.server
    );
    println!("  Channel        : {}/{}/channel", org, ns);
    println!("  Total messages : {}", comma_format(args.msgs as i64));
    println!("  Payload size   : {} bytes", args.size);
    println!();

    let payload = vec![0u8; args.size];

    let client_config =
        ClientConfig::with_endpoint(&args.server).with_tls_setting(TlsClientConfig::insecure());
    let conn_id = service
        .connect(&client_config)
        .await
        .context("connect to server failed")?;

    match run_channel_pub_worker(
        service,
        &org,
        &ns,
        &args.secret,
        conn_id,
        args.count,
        args.msgs,
        payload,
    )
    .await
    {
        Ok(sample) => {
            let mut group = SampleGroup::new();
            group.add(sample);
            println!("\n{}", group.report("Channel pub stats"));
            if let Some(csv_path) = &args.csv {
                let run_id = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
                append_csv(csv_path, &group.csv_rows(&run_id, "CP"))
                    .with_context(|| format!("writing CSV to {csv_path}"))?;
                println!("Results written to {csv_path}");
            }
        }
        Err(e) => eprintln!("[ch-pub] error: {e:#}"),
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_channel_pub_worker(
    service: &Service,
    org: &str,
    ns: &str,
    secret: &str,
    conn_id: u64,
    sub_count: usize,
    msg_count: u64,
    payload: Vec<u8>,
) -> Result<Sample> {
    let own_name = make_channel_pub_name(org, ns);
    let channel_name = make_channel_name(org, ns);

    let (app, _notification_rx) = create_app_with_secret(service, &own_name, secret).await?;

    app.subscribe(&own_name, Some(conn_id))
        .await
        .context("subscribe failed")?;

    for i in 0..sub_count {
        let sub_name = make_channel_sub_name(org, ns, i);
        app.set_route(&sub_name, conn_id)
            .await
            .context("set route to subscriber failed")?;
    }

    println!("[ch-pub] subscribed {own_name} — creating group session on {channel_name}...");

    let session_ctx = create_and_wait_session(
        &app,
        ProtoSessionType::Multicast,
        channel_name.clone(),
        Duration::from_secs(5),
        "group session creation timed out unexpectedly",
    )
    .await?;

    let (weak_session, _session_rx) = session_ctx.into_parts();
    let controller = weak_session
        .upgrade()
        .ok_or_else(|| anyhow!("session closed"))?;
    println!("[ch-pub] group session created on {channel_name}");

    for i in 0..sub_count {
        let sub_name = make_channel_sub_name(org, ns, i);
        println!("[ch-pub] inviting {sub_name}...");
        tokio::time::timeout(SESSION_ESTABLISH_TIMEOUT, async {
            controller.invite_participant(&sub_name).await?.await
        })
        .await
        .with_context(|| {
            format!(
                "[ch-pub] invite of {sub_name} timed out — \
                 is `slimctl bench channel sub` running with matching --prefix and --count?"
            )
        })?
        .with_context(|| format!("invite of {sub_name} failed"))?;
        println!("[ch-pub] {sub_name} joined");
    }

    println!("[ch-pub] all {sub_count} subscriber(s) joined — publishing {msg_count} messages");

    let start = Instant::now();
    let msg_bytes = msg_count * payload.len() as u64;

    for _ in 0..msg_count {
        controller
            .publish(controller.dst(), payload.clone(), None, None)
            .await
            .context("publish failed")?
            .await
            .context("publish completion failed")?;
    }

    let end = Instant::now();

    Ok(Sample {
        job_msg_cnt: msg_count,
        msg_cnt: msg_count,
        msg_bytes,
        start,
        end,
    })
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

/// Parse a human-readable byte size string into a `usize`.
/// Accepts plain integers (`128`) or a number followed by an optional unit
/// suffix (case-insensitive, with or without space).
///
/// SI (decimal, powers of 1000): kb, mb, gb
/// IEC (binary, powers of 1024): kib, mib, gib
/// Bare `b` = bytes (× 1)
///
/// Examples: "128", "16kb", "16kib", "4 MB", "1GiB"
fn parse_byte_size(s: &str) -> Result<usize, String> {
    let s = s.trim();
    if let Ok(n) = s.parse::<usize>() {
        return Ok(n);
    }
    let split = s
        .find(|c: char| !c.is_ascii_digit())
        .ok_or_else(|| format!("invalid size: '{s}'"))?;
    let (num_str, suffix) = s.split_at(split);
    let n: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid size: '{s}'"))?;
    let multiplier: u64 = match suffix.trim().to_lowercase().as_str() {
        "b" => 1,
        "kb" => 1_000,
        "mb" => 1_000_000,
        "gb" => 1_000_000_000,
        "kib" => 1_024,
        "mib" => 1_024 * 1_024,
        "gib" => 1_024 * 1_024 * 1_024,
        other => {
            return Err(format!(
                "unknown size unit: '{other}' (use b/kb/mb/gb or kib/mib/gib)"
            ));
        }
    };
    usize::try_from(n * multiplier).map_err(|_| format!("size too large: '{s}'"))
}

fn human_bytes(bytes: f64) -> String {
    const BASE: f64 = 1024.0;
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
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
        assert_eq!(sub.str_components(), ("bench", "test", "sub-3"));

        let pub_ = make_pub_name("bench", "test", 0);
        assert_eq!(pub_.str_components(), ("bench", "test", "pub-0"));
    }

    #[test]
    fn make_channel_names_correct() {
        let ch = make_channel_name("bench", "test");
        assert_eq!(ch.str_components(), ("bench", "test", "channel"));

        let pub_ = make_channel_pub_name("bench", "test");
        assert_eq!(pub_.str_components(), ("bench", "test", "channel-pub"));

        let sub = make_channel_sub_name("bench", "test", 2);
        assert_eq!(sub.str_components(), ("bench", "test", "channel-sub-2"));
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
        assert!(human_bytes(1500.0).contains("KiB"));
        assert!(human_bytes(2.0 * 1024.0 * 1024.0).contains("MiB"));
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
        let samples: Vec<Duration> = (1..=100).map(Duration::from_micros).collect();
        let lr = LatencyReport::new(samples);
        assert_eq!(lr.percentile(50.0), Duration::from_micros(50));
        assert_eq!(lr.percentile(99.0), Duration::from_micros(99));
        assert!(!lr.report().is_empty());
    }
}
