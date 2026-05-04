// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Shared setup for `process_stream` benchmarks (Criterion timing and allocation harness).

use std::sync::OnceLock;
use std::time::Duration;

use slim_datapath::api::ProtoMessage;
use slim_datapath::message_processing::MessageProcessor;
use slim_datapath::messages::Name;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;

pub fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().expect("tokio runtime"))
}

pub fn bench_destination() -> Name {
    Name::from_strings(["org", "ns", "bench-dst"]).with_id(2)
}

/// Inbound subscribe from a remote peer (valid SLIM header so rebuild returns `Some`).
pub fn make_remote_subscribe_for_cp_mirror() -> ProtoMessage {
    let source = Name::from_strings(["org", "ns", "bench-src"]).with_id(1);
    ProtoMessage::builder()
        .source(source)
        .destination(bench_destination())
        .subscription_id(42)
        .build_subscribe()
        .expect("valid subscribe for bench")
}

pub async fn assert_cp_mirror_rebuild_path() {
    let processor = MessageProcessor::new();
    let (cp_tx, mut cp_rx) = mpsc::channel(8);
    processor.bench_set_control_plane_tx(cp_tx);
    let conn_id = processor.bench_register_remote_connection();
    let (in_tx, in_rx) = mpsc::channel(4);
    let cancel = CancellationToken::new();
    let handle = processor
        .bench_process_stream(
            ReceiverStream::new(in_rx),
            conn_id,
            None,
            cancel.clone(),
            false,
            false,
        )
        .expect("bench_process_stream");

    let msg = make_remote_subscribe_for_cp_mirror();
    let expected_dst = msg.get_dst();
    in_tx.send(Ok(msg)).await.expect("send inbound");
    let mirrored = cp_rx
        .recv()
        .await
        .expect("control plane should receive message")
        .expect("mirror Ok");

    assert_eq!(
        mirrored.get_dst(),
        expected_dst,
        "mirrored message must carry subscription destination for the control plane"
    );
    assert!(
        mirrored.get_subscription_id().is_some(),
        "mirrored subscribe should carry subscription_id"
    );

    cancel.cancel();
    match tokio::time::timeout(Duration::from_secs(2), handle).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => panic!("bench_process_stream task failed: {e}"),
        Err(_) => panic!("join timeout"),
    }
}

pub async fn one_iteration_cp_mirror() {
    let processor = MessageProcessor::new();
    let (cp_tx, mut cp_rx) = mpsc::channel(8);
    processor.bench_set_control_plane_tx(cp_tx);
    let conn_id = processor.bench_register_remote_connection();
    let (in_tx, in_rx) = mpsc::channel(4);
    let cancel = CancellationToken::new();
    let handle = processor
        .bench_process_stream(
            ReceiverStream::new(in_rx),
            conn_id,
            None,
            cancel.clone(),
            false,
            false,
        )
        .expect("bench_process_stream");

    let msg = make_remote_subscribe_for_cp_mirror();
    in_tx.send(Ok(msg)).await.expect("send inbound");
    let _ = cp_rx.recv().await;
    cancel.cancel();
    let _join: Result<Result<(), tokio::task::JoinError>, tokio::time::error::Elapsed> =
        tokio::time::timeout(Duration::from_secs(2), handle).await;
}
