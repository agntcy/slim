// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! In-process session-layer stress test — exercises the full session pipeline
//! (App, SessionController, HMAC signing, sequence numbers) without network.
//!
//! Usage:
//!   stress-session --senders N --messages M --payload-size B [--unreliable]

use std::time::Duration;

use clap::Parser;
use slim_testing::stress::BenchmarkConfig;
use slim_testing::stress_session::run_session_benchmark;

#[derive(Parser, Debug)]
#[command(about = "In-process session-layer stress test for SLIM (bypasses gRPC)")]
struct Args {
    /// Number of concurrent sender tasks
    #[arg(long, default_value_t = 8)]
    senders: usize,

    /// Messages per sender
    #[arg(long, default_value_t = 100_000)]
    messages: u64,

    /// Payload size in bytes
    #[arg(long, default_value_t = 64)]
    payload_size: usize,

    /// Drain timeout in seconds
    #[arg(long, default_value_t = 5)]
    drain_timeout: u64,

    /// Run in unreliable (fire-and-forget) mode.
    ///
    /// Sessions have no retransmit timers and senders apply no backpressure.
    /// This measures raw session-layer send throughput but may report less
    /// than 100% delivery when the outbound channel saturates under high load.
    /// By default the benchmark runs in reliable mode (ACK-gated semaphore),
    /// which guarantees 100% delivery.
    #[arg(long, default_value_t = false)]
    unreliable: bool,

    /// Config file (optional, only used for tracing setup)
    #[arg(short, long)]
    config: Option<String>,

    /// E2E header validation percentage (0-100) to enable MLS.
    /// If not specified, MLS is completely disabled.
    #[arg(long)]
    validation_percent: Option<u32>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let reliable = !args.unreliable;

    // Minimal tracing — perf test only needs warn/error output.
    if let Some(config_file) = &args.config {
        let mut loader =
            slim::config::ConfigLoader::new(config_file).expect("failed to load configuration");
        let _guard = loader
            .tracing()
            .expect("invalid tracing configuration")
            .setup_tracing_subscriber();
    }

    let cfg = BenchmarkConfig {
        senders: args.senders,
        messages: args.messages,
        payload_size: args.payload_size,
        drain_timeout: Duration::from_secs(args.drain_timeout),
        validation_percent: args.validation_percent,
    };

    println!("Starting in-process session-layer stress test:");
    println!("  Senders:         {}", cfg.senders);
    println!("  Messages/sender: {}", cfg.messages);
    println!("  Payload size:    {} bytes", cfg.payload_size);
    println!("  Total messages:  {}", cfg.senders as u64 * cfg.messages);
    println!("  Drain timeout:   {}s", cfg.drain_timeout.as_secs());
    println!(
        "  Mode:            {}",
        if reliable { "reliable" } else { "unreliable" }
    );
    if let Some(percent) = cfg.validation_percent {
        println!(
            "  MLS:             enabled (E2E header validation: {}%)",
            percent
        );
    } else {
        println!("  MLS:             disabled");
    }
    println!();

    let result = run_session_benchmark(&cfg, reliable)
        .await
        .map_err(|e| format!("{e}"))?;

    println!("====== SLIM Session-Layer Stress Test Results ======");
    println!("  Senders:           {}", cfg.senders);
    println!("  Messages/sender:   {}", cfg.messages);
    println!("  Payload size:      {} bytes", cfg.payload_size);
    println!(
        "  Mode:              {}",
        if reliable { "reliable" } else { "unreliable" }
    );
    if let Some(percent) = cfg.validation_percent {
        println!(
            "  MLS:               enabled (E2E header validation: {}%)",
            percent
        );
    } else {
        println!("  MLS:               disabled");
    }
    println!("  Total sent:        {}", result.total_sent);
    println!("  Total received:    {}", result.total_received);
    println!("  Delivery:          {:.1}%", result.delivery_pct());
    println!(
        "  Send elapsed:      {:.3}s",
        result.send_elapsed.as_secs_f64()
    );
    println!(
        "  Total elapsed:     {:.3}s",
        result.total_elapsed.as_secs_f64()
    );
    println!("  Throughput (send): {:.0} msg/s", result.send_mps());
    println!("  Throughput (recv): {:.0} msg/s", result.recv_mps());
    println!("====================================================");

    Ok(())
}
