// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! In-process session-layer benchmark library.
//!
//! Provides [`run_session_benchmark`] which exercises the full session pipeline
//! (App → SessionController → MessageProcessor) without any network transport,
//! measuring throughput through HMAC signing, session headers, and sequence
//! number management.
//!
//! Uses the same **dumbbell topology** as the datapath benchmark: each of the
//! `cfg.senders` concurrent sender tasks is paired with its own dedicated
//! receiver app over an independent point-to-point session.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use slim_auth::shared_secret::SharedSecret;
use slim_config::component::id::{ID, Kind};
use slim_datapath::api::{ProtoName as Name, ProtoSessionType};
use slim_service::Service;
use slim_session::{Notification, SessionConfig};
use tracing::info;

use crate::stress::{BenchmarkConfig, BenchmarkResult};
use crate::utils::TEST_VALID_SECRET;

/// Run an in-process session-layer benchmark against a freshly created [`Service`].
///
/// Creates N sender/receiver [`App`] pairs on a single in-memory service (no
/// network). Each pair establishes a point-to-point session before the timed
/// phase begins, so only message publishing throughput is measured.
///
/// # Reliable vs unreliable mode
///
/// When `reliable` is `true`, sessions are configured with retransmit timers
/// (`max_retries = 10`, `interval = 1 s`). The session layer will ACK received
/// messages and retransmit any that are dropped by the bounded outbound channel.
///
/// When `reliable` is `false`, sessions use the default fire-and-forget
/// configuration (no timers, no ACKs). Messages dropped by the outbound
/// channel are permanently lost.
///
/// In both modes senders publish without external throttling, so the results
/// reflect the session layer's own behaviour under load.
///
/// Returns aggregate throughput metrics once all senders finish and the drain
/// timeout elapses.
pub async fn run_session_benchmark(
    cfg: &BenchmarkConfig,
    reliable: bool,
) -> Result<BenchmarkResult, Box<dyn std::error::Error + Send + Sync>> {
    // Early return for edge cases — no service needed.
    if cfg.senders == 0 || cfg.messages == 0 {
        return Ok(BenchmarkResult {
            total_sent: cfg.senders as u64 * cfg.messages,
            total_received: 0,
            send_elapsed: Duration::ZERO,
            total_elapsed: Duration::ZERO,
        });
    }

    // Create a single in-memory service with no network connections.
    // All apps share the same MessageProcessor — subscriptions are visible
    // across apps on the same service without explicit subscribe calls.
    let service =
        Service::new(ID::new_with_name(Kind::new("slim").unwrap(), "stress-session").unwrap());

    // Per-lane receive counters.
    let received_counts: Vec<Arc<AtomicU64>> = (0..cfg.senders)
        .map(|_| Arc::new(AtomicU64::new(0)))
        .collect();

    // Vecs to hold apps (must stay alive for the duration of the benchmark)
    // and notification channels for the receiver tasks.
    let mut receiver_apps = Vec::with_capacity(cfg.senders);
    let mut sender_apps = Vec::with_capacity(cfg.senders);
    let mut notification_rxs = Vec::with_capacity(cfg.senders);
    let mut receiver_dest_names = Vec::with_capacity(cfg.senders);

    // Set up N independent (sender_i → session_i → receiver_i) lanes.
    // Each lane uses a unique 3-component name so routing is exclusive.
    for i in 0..cfg.senders {
        let recv_name = Name::from_strings(["perf", "stress", &format!("recv{i}")]);
        let send_name = Name::from_strings(["perf", "stress", &format!("send{i}")]);

        let (receiver_app, notification_rx) = service
            .create_app(
                &recv_name,
                SharedSecret::new(&format!("receiver-{i}"), TEST_VALID_SECRET)?,
                SharedSecret::new(&format!("receiver-{i}"), TEST_VALID_SECRET)?,
            )
            .map_err(|e| format!("failed to create receiver app {i}: {e}"))?;

        let (sender_app, _sender_rx) = service
            .create_app(
                &send_name,
                SharedSecret::new(&format!("sender-{i}"), TEST_VALID_SECRET)?,
                SharedSecret::new(&format!("sender-{i}"), TEST_VALID_SECRET)?,
            )
            .map_err(|e| format!("failed to create sender app {i}: {e}"))?;

        // The session destination is the 3-component receiver name (no ID).
        // Since each lane has a unique third component ("recv{i}"), routing
        // is exclusive to the paired receiver.
        receiver_dest_names.push(recv_name);
        notification_rxs.push(notification_rx);
        receiver_apps.push(receiver_app);
        sender_apps.push(sender_app);
    }

    // Allow all app subscriptions to propagate through the processing loop.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Spawn receiver tasks BEFORE session creation so they are ready to
    // consume the NewSession notification triggered by the sender's
    // DiscoveryRequest.
    let mut recv_handles = Vec::with_capacity(cfg.senders);
    for (i, (mut notification_rx, count)) in notification_rxs
        .into_iter()
        .zip(received_counts.iter().cloned())
        .enumerate()
    {
        let handle = tokio::spawn(async move {
            // Wait for the NewSession notification.
            let notification = match notification_rx.recv().await {
                Some(Ok(n)) => n,
                Some(Err(e)) => {
                    tracing::error!(lane = i, "receiver notification error: {e}");
                    return;
                }
                None => {
                    tracing::error!(
                        lane = i,
                        "receiver notification channel closed unexpectedly"
                    );
                    return;
                }
            };

            let mut recv_ctx = match notification {
                Notification::NewSession(ctx) => ctx,
                _ => {
                    tracing::error!(lane = i, "expected NewSession notification");
                    return;
                }
            };

            // Count messages as they arrive on this session's rx channel.
            while let Some(Ok(_msg)) = recv_ctx.rx.recv().await {
                count.fetch_add(1, Ordering::Relaxed);
            }
        });
        recv_handles.push(handle);
    }

    // Create all sessions and collect completion handles, then await them
    // all. Session establishment is NOT timed — only message publishing is.
    let mut session_contexts = Vec::with_capacity(cfg.senders);
    let mut completion_handles = Vec::with_capacity(cfg.senders);
    for i in 0..cfg.senders {
        let mut config = SessionConfig::default().with_session_type(ProtoSessionType::PointToPoint);
        config.initiator = true;
        if reliable {
            // Both max_retries AND interval must be Some for the session to
            // create a timer factory. Without both the session runs in
            // unreliable mode: CompletionHandles resolve immediately and no
            // ACKs are sent, making semaphore-based backpressure ineffective.
            config.max_retries = Some(10);
            config.interval = Some(Duration::from_secs(1));
        }
        if let Some(percent) = cfg.validation_percent {
            config.mls_settings = Some(slim_session::session_config::MlsSettings {
                header_integrity_validation_percent: percent,
                max_seen_control_message_ids_size: None,
            });
        }

        let dest = receiver_dest_names[i].clone();
        let (session_ctx, completion_handle) = sender_apps[i]
            .create_session(config, dest, None)
            .await
            .map_err(|e| format!("failed to create session for lane {i}: {e}"))?;

        session_contexts.push(session_ctx);
        completion_handles.push(completion_handle);
    }

    // Wait for all sessions to be established before starting the clock.
    for (i, handle) in completion_handles.into_iter().enumerate() {
        handle
            .await
            .map_err(|e| format!("session {i} establishment failed: {e}"))?;
    }

    // Pre-build the payload blob once; sender tasks clone it per message.
    let payload_blob = vec![0xABu8; cfg.payload_size];
    let total_sent = cfg.senders as u64 * cfg.messages;

    let start = Instant::now();

    // Spawn concurrent sender tasks — one per lane, fire-and-forget.
    // The CompletionHandle is discarded so the sender never waits for an ACK
    // before sending the next message, letting the session layer's own
    // behaviour under load be observable without external throttling.
    let mut sender_handles = Vec::with_capacity(cfg.senders);
    for (i, session_ctx) in session_contexts.into_iter().enumerate() {
        let msg_count = cfg.messages;
        let payload = payload_blob.clone();
        let dest = receiver_dest_names[i].clone();

        let handle = tokio::spawn(async move {
            let session = session_ctx
                .session_arc()
                .expect("session should be alive during benchmark");

            for _ in 0..msg_count {
                match session.publish(&dest, payload.clone(), None, None).await {
                    Ok(_completion) => {}
                    Err(e) => {
                        tracing::error!(sender = i, "publish error: {e}");
                        break;
                    }
                }
            }
            info!(sender = i, "sender done");
        });
        sender_handles.push(handle);
    }

    for h in sender_handles {
        h.await?;
    }

    let send_elapsed = start.elapsed();

    // Actively poll until all in-flight messages are received or drain_timeout
    // elapses — same drain strategy as the datapath benchmark.
    let drain_deadline = tokio::time::Instant::now() + cfg.drain_timeout;
    loop {
        let received_so_far: u64 = received_counts
            .iter()
            .map(|c| c.load(Ordering::Relaxed))
            .sum();
        if received_so_far >= total_sent || tokio::time::Instant::now() >= drain_deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    let total_elapsed = start.elapsed();
    let total_received: u64 = received_counts
        .iter()
        .map(|c| c.load(Ordering::Relaxed))
        .sum();

    // Abort receiver tasks (they are blocked in their receive loop).
    for h in recv_handles {
        h.abort();
    }
    // Drop apps to release their MessageProcessor connections.
    drop(sender_apps);
    drop(receiver_apps);

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
            // Generous timeout: session layer has higher per-message overhead
            // than the raw datapath benchmark.
            drain_timeout: Duration::from_secs(5),
            validation_percent: None,
        }
    }

    // -----------------------------------------------------------------------
    // Routing and delivery
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn single_sender_full_delivery() {
        let cfg = quick_cfg(1, 50, 64);
        let r = run_session_benchmark(&cfg, true).await.unwrap();
        assert_eq!(r.total_sent, 50);
        assert_eq!(r.total_received, 50);
        assert_eq!(r.delivery_pct(), 100.0);
    }

    #[tokio::test]
    async fn multi_sender_full_delivery() {
        let cfg = quick_cfg(4, 20, 64);
        let r = run_session_benchmark(&cfg, true).await.unwrap();
        assert_eq!(r.total_sent, 80);
        assert_eq!(r.total_received, 80);
        assert_eq!(r.delivery_pct(), 100.0);
    }

    // -----------------------------------------------------------------------
    // Payload sizes
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn large_payload_delivered() {
        let cfg = quick_cfg(1, 5, 4096);
        let r = run_session_benchmark(&cfg, true).await.unwrap();
        assert_eq!(r.total_sent, 5);
        assert_eq!(r.total_received, 5);
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn zero_messages_no_delivery() {
        let cfg = quick_cfg(4, 0, 64);
        let r = run_session_benchmark(&cfg, true).await.unwrap();
        assert_eq!(r.total_sent, 0);
        assert_eq!(r.total_received, 0);
    }

    #[tokio::test]
    async fn zero_senders_no_delivery() {
        let cfg = quick_cfg(0, 100, 64);
        let r = run_session_benchmark(&cfg, true).await.unwrap();
        assert_eq!(r.total_sent, 0);
        assert_eq!(r.total_received, 0);
    }
}
