// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Measures [`MessageProcessor::bench_process_stream`] when remote subscribe/unsubscribe is mirrored
//! to the control plane via [`ProtoMessage::rebuild_header_for_control_plane`] (compact header +
//! `recv_from` = dataplane connection index), with `unwrap_or_else(|| msg.clone())` only as fallback.
//!
//! Setup: `is_local = false`, `from_control_plane = false`, mock CP channel from
//! [`MessageProcessor::bench_set_control_plane_tx`].
//!
//! For heap allocation counters over the same workload, run
//! `cargo bench -p agntcy-slim-datapath --bench process_stream_alloc` or
//! `task data-plane:bench:process-stream`.

mod process_stream_common;

use criterion::{Criterion, black_box, criterion_group, criterion_main};

fn bench_process_stream_cp_mirror_rebuild(c: &mut Criterion) {
    let rt = process_stream_common::runtime();
    rt.block_on(process_stream_common::assert_cp_mirror_rebuild_path());

    c.bench_function("process_stream: CP mirror (rebuild_header)", |b| {
        b.iter(|| {
            rt.block_on(black_box(process_stream_common::one_iteration_cp_mirror()));
        });
    });
}

criterion_group!(benches, bench_process_stream_cp_mirror_rebuild);
criterion_main!(benches);
