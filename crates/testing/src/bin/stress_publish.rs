// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! In-process datapath stress test — bypasses gRPC/H2 transport entirely
//! by using register_local_connection to inject messages directly into
//! the SLIM MessageProcessor via mpsc channels.
//!
//! This measures the raw forwarding hot-path throughput.
//!
//! Usage:
//!   stress-publish --senders N --messages M --payload-size B

use clap::Parser;
use slim_testing::stress::{BenchmarkConfig, run_benchmark};

#[derive(Parser, Debug)]
#[command(about = "In-process datapath stress test for SLIM (bypasses gRPC)")]
struct Args {
    /// Number of concurrent sender tasks
    #[arg(long, default_value_t = 8)]
    senders: usize,

    /// Messages per sender
    #[arg(long, default_value_t = 1_000_000)]
    messages: u64,

    /// Payload size in bytes
    #[arg(long, default_value_t = 64)]
    payload_size: usize,

    /// Config file (optional, only used for tracing setup)
    #[arg(short, long)]
    config: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Minimal tracing — perf test only needs warn/error output
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
        ..Default::default()
    };

    println!("Starting in-process stress test:");
    println!("  Senders:         {}", cfg.senders);
    println!("  Messages/sender: {}", cfg.messages);
    println!("  Payload size:    {} bytes", cfg.payload_size);
    println!("  Total messages:  {}", cfg.senders as u64 * cfg.messages);
    println!();

    let result = run_benchmark(&cfg).await.map_err(|e| format!("{e}"))?;

    println!("====== SLIM In-Process Stress Test Results ======");
    println!("  Senders:           {}", cfg.senders);
    println!("  Messages/sender:   {}", cfg.messages);
    println!("  Payload size:      {} bytes", cfg.payload_size);
    println!("  Total sent:        {}", result.total_sent);
    println!("  Total received:    {}", result.total_received);
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
    println!("=================================================");

    Ok(())
}
