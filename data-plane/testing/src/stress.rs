// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! In-process datapath benchmark library.
//!
//! Provides [`run_benchmark`] which bypasses gRPC/H2 transport by injecting
//! messages directly into a [`MessageProcessor`] via mpsc channels, exercising
//! the raw forwarding hot-path.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use slim_datapath::api::ApplicationPayload;
use slim_datapath::api::ProtoMessage as Message;
use slim_datapath::message_processing::MessageProcessor;
use slim_datapath::messages::Name;
use tracing::info;

/// Configuration for a single in-process benchmark run.
#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    /// Number of concurrent sender tasks.
    pub senders: usize,
    /// Number of messages each sender publishes.
    pub messages: u64,
    /// Payload size in bytes for each message.
    pub payload_size: usize,
    /// How long to wait after senders finish for the receiver to drain.
    /// Defaults to 2 seconds; tests may use a smaller value.
    pub drain_timeout: Duration,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            senders: 8,
            messages: 1_000_000,
            payload_size: 64,
            drain_timeout: Duration::from_secs(2),
        }
    }
}

/// Results from a completed benchmark run.
#[derive(Debug)]
pub struct BenchmarkResult {
    /// Total messages sent by all senders.
    pub total_sent: u64,
    /// Total messages received by the receiver task.
    pub total_received: u64,
    /// Elapsed time until all senders finished.
    pub send_elapsed: Duration,
    /// Total elapsed time including the drain wait.
    pub total_elapsed: Duration,
}

impl BenchmarkResult {
    /// Messages sent per second (based on send elapsed time).
    pub fn send_mps(&self) -> f64 {
        if self.send_elapsed.is_zero() {
            return 0.0;
        }
        self.total_sent as f64 / self.send_elapsed.as_secs_f64()
    }

    /// Messages received per second (based on total elapsed time).
    pub fn recv_mps(&self) -> f64 {
        if self.total_elapsed.is_zero() {
            return 0.0;
        }
        self.total_received as f64 / self.total_elapsed.as_secs_f64()
    }

    /// Delivery percentage (0.0–100.0). Returns 100.0 when nothing was sent.
    pub fn delivery_pct(&self) -> f64 {
        if self.total_sent == 0 {
            return 100.0;
        }
        self.total_received as f64 / self.total_sent as f64 * 100.0
    }
}

/// Run an in-process benchmark against a freshly created [`MessageProcessor`].
///
/// Sets up one receiver subscribed to the publish topic, then spawns
/// `cfg.senders` concurrent sender tasks each publishing `cfg.messages`
/// messages of `cfg.payload_size` bytes. Returns throughput metrics once all
/// senders complete and the drain timeout elapses.
pub async fn run_benchmark(
    cfg: &BenchmarkConfig,
) -> Result<BenchmarkResult, Box<dyn std::error::Error + Send + Sync>> {
    let mp = MessageProcessor::new();
    let dest_type = Name::from_strings(["perf", "stress", "topic"]);

    // Register receiver and subscribe it to the topic.
    let (_recv_conn_id, recv_tx, mut recv_rx) = mp
        .register_local_connection(false)
        .expect("failed to register receiver connection");

    let sub_msg = Message::builder()
        .source(dest_type.clone())
        .destination(dest_type.clone())
        .build_subscribe()
        .unwrap();
    recv_tx.send(Ok(sub_msg)).await?;

    // Allow subscription to propagate through the processing loop.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Register sender connections — senders do NOT subscribe to dest_type
    // to avoid messages being load-balanced back to other senders.
    let mut sender_channels = Vec::new();
    for i in 0..cfg.senders {
        let sender_name = Name::from_strings(["perf", "stress", "sender"]).with_id(i as u64);
        let (_conn_id, tx, _rx) = mp
            .register_local_connection(false)
            .expect("failed to register sender connection");
        sender_channels.push((i, sender_name, tx, _rx));
    }

    // Allow all registrations to settle.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Pre-build the payload content once; all messages share the same blob.
    let payload_blob = vec![0xABu8; cfg.payload_size];
    let content = ApplicationPayload::new("perf", payload_blob).as_content();

    let total_sent = cfg.senders as u64 * cfg.messages;
    let received_count = Arc::new(AtomicU64::new(0));

    // Receiver drain task: count every successfully received message.
    let recv_count = received_count.clone();
    let recv_handle = tokio::spawn(async move {
        while let Some(msg) = recv_rx.recv().await {
            if msg.is_ok() {
                recv_count.fetch_add(1, Ordering::Relaxed);
            }
        }
    });

    let start = Instant::now();

    // Spawn concurrent sender tasks.
    let mut handles = Vec::new();
    for (sender_id, sender_name, tx, _rx) in sender_channels {
        let msg_count = cfg.messages;
        let content_clone = content.clone();
        let dest = dest_type.clone();

        let handle = tokio::spawn(async move {
            for _ in 0..msg_count {
                let msg = Message::builder()
                    .source(sender_name.clone())
                    .destination(dest.clone())
                    .payload(content_clone.clone())
                    .build_publish()
                    .unwrap();

                if tx.send(Ok(msg)).await.is_err() {
                    break;
                }
            }
            info!(sender = sender_id, "sender done");
        });
        handles.push(handle);
    }

    for h in handles {
        h.await?;
    }

    let send_elapsed = start.elapsed();

    // Give the receiver time to drain the remaining in-flight messages.
    tokio::time::sleep(cfg.drain_timeout).await;

    let total_elapsed = start.elapsed();
    let total_received = received_count.load(Ordering::Relaxed);

    recv_handle.abort();
    mp.shutdown().await.ok();

    Ok(BenchmarkResult {
        total_sent,
        total_received,
        send_elapsed,
        total_elapsed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quick_cfg(senders: usize, messages: u64, payload_size: usize) -> BenchmarkConfig {
        BenchmarkConfig {
            senders,
            messages,
            payload_size,
            drain_timeout: Duration::from_millis(500),
        }
    }

    // -----------------------------------------------------------------------
    // Routing and delivery
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn single_sender_full_delivery() {
        let cfg = quick_cfg(1, 100, 64);
        let r = run_benchmark(&cfg).await.unwrap();
        assert_eq!(r.total_sent, 100);
        assert_eq!(r.total_received, 100);
        assert_eq!(r.delivery_pct(), 100.0);
    }

    #[tokio::test]
    async fn multi_sender_full_delivery() {
        let cfg = quick_cfg(4, 50, 64);
        let r = run_benchmark(&cfg).await.unwrap();
        assert_eq!(r.total_sent, 200);
        assert_eq!(r.total_received, 200);
        assert_eq!(r.delivery_pct(), 100.0);
    }

    #[tokio::test]
    async fn many_senders_full_delivery() {
        let cfg = quick_cfg(8, 25, 64);
        let r = run_benchmark(&cfg).await.unwrap();
        assert_eq!(r.total_sent, 200);
        assert_eq!(r.total_received, 200);
    }

    // -----------------------------------------------------------------------
    // Payload sizes
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn zero_byte_payload_delivered() {
        let cfg = quick_cfg(1, 20, 0);
        let r = run_benchmark(&cfg).await.unwrap();
        assert_eq!(r.total_received, 20);
    }

    #[tokio::test]
    async fn small_payload_delivered() {
        let cfg = quick_cfg(2, 10, 1);
        let r = run_benchmark(&cfg).await.unwrap();
        assert_eq!(r.total_received, 20);
    }

    #[tokio::test]
    async fn large_payload_delivered() {
        let cfg = quick_cfg(1, 5, 65536);
        let r = run_benchmark(&cfg).await.unwrap();
        assert_eq!(r.total_received, 5);
    }

    #[tokio::test]
    async fn medium_payload_delivered() {
        let cfg = quick_cfg(2, 20, 1024);
        let r = run_benchmark(&cfg).await.unwrap();
        assert_eq!(r.total_received, 40);
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn zero_messages_no_delivery() {
        let cfg = quick_cfg(4, 0, 64);
        let r = run_benchmark(&cfg).await.unwrap();
        assert_eq!(r.total_sent, 0);
        assert_eq!(r.total_received, 0);
    }

    #[tokio::test]
    async fn zero_senders_no_delivery() {
        let cfg = quick_cfg(0, 100, 64);
        let r = run_benchmark(&cfg).await.unwrap();
        assert_eq!(r.total_sent, 0);
        assert_eq!(r.total_received, 0);
    }

    #[tokio::test]
    async fn single_message_delivered() {
        let cfg = quick_cfg(1, 1, 256);
        let r = run_benchmark(&cfg).await.unwrap();
        assert_eq!(r.total_sent, 1);
        assert_eq!(r.total_received, 1);
    }

    // -----------------------------------------------------------------------
    // BenchmarkResult computed fields
    // -----------------------------------------------------------------------

    #[test]
    fn delivery_pct_zero_sent_is_100() {
        let r = BenchmarkResult {
            total_sent: 0,
            total_received: 0,
            send_elapsed: Duration::from_millis(1),
            total_elapsed: Duration::from_millis(1),
        };
        assert_eq!(r.delivery_pct(), 100.0);
    }

    #[test]
    fn delivery_pct_partial() {
        let r = BenchmarkResult {
            total_sent: 200,
            total_received: 100,
            send_elapsed: Duration::from_secs(1),
            total_elapsed: Duration::from_secs(2),
        };
        assert_eq!(r.delivery_pct(), 50.0);
    }

    #[test]
    fn send_mps_calculation() {
        let r = BenchmarkResult {
            total_sent: 1000,
            total_received: 1000,
            send_elapsed: Duration::from_secs(1),
            total_elapsed: Duration::from_secs(2),
        };
        assert_eq!(r.send_mps(), 1000.0);
        assert_eq!(r.recv_mps(), 500.0);
    }

    #[test]
    fn send_mps_zero_elapsed_returns_zero() {
        let r = BenchmarkResult {
            total_sent: 100,
            total_received: 100,
            send_elapsed: Duration::ZERO,
            total_elapsed: Duration::ZERO,
        };
        assert_eq!(r.send_mps(), 0.0);
        assert_eq!(r.recv_mps(), 0.0);
    }

    #[test]
    fn benchmark_result_debug() {
        let r = BenchmarkResult {
            total_sent: 10,
            total_received: 10,
            send_elapsed: Duration::from_millis(10),
            total_elapsed: Duration::from_millis(20),
        };
        // Ensure Debug derives don't panic
        let s = format!("{:?}", r);
        assert!(s.contains("total_sent"));
    }

    #[test]
    fn benchmark_config_default() {
        let cfg = BenchmarkConfig::default();
        assert_eq!(cfg.senders, 8);
        assert_eq!(cfg.messages, 1_000_000);
        assert_eq!(cfg.payload_size, 64);
        assert_eq!(cfg.drain_timeout, Duration::from_secs(2));
    }

    #[test]
    fn benchmark_config_clone() {
        let cfg = quick_cfg(2, 50, 128);
        let cloned = cfg.clone();
        assert_eq!(cloned.senders, 2);
        assert_eq!(cloned.messages, 50);
        assert_eq!(cloned.payload_size, 128);
    }
}
