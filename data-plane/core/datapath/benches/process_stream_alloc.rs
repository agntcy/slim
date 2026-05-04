// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Allocation counters for one `bench_process_stream` iteration (CP mirror path), using
//! [`stats_alloc`](https://docs.rs/stats_alloc).

mod process_stream_common;

use std::alloc::System;

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn main() {
    let rt = process_stream_common::runtime();
    rt.block_on(process_stream_common::assert_cp_mirror_rebuild_path());

    const WARMUP: usize = 3;
    for _ in 0..WARMUP {
        rt.block_on(process_stream_common::one_iteration_cp_mirror());
    }

    const SAMPLES: usize = 50;
    let mut allocation_counts = Vec::with_capacity(SAMPLES);
    let mut bytes_allocated = Vec::with_capacity(SAMPLES);
    let mut deallocations = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let region = Region::new(GLOBAL);
        rt.block_on(process_stream_common::one_iteration_cp_mirror());
        let s = region.change();
        allocation_counts.push(s.allocations);
        bytes_allocated.push(s.bytes_allocated);
        deallocations.push(s.deallocations);
    }

    fn summarize(values: &[usize]) -> (usize, usize, f64) {
        let min = *values.iter().min().unwrap_or(&0);
        let max = *values.iter().max().unwrap_or(&0);
        let mean = values.iter().sum::<usize>() as f64 / values.len().max(1) as f64;
        (min, max, mean)
    }

    let (a_min, a_max, a_mean) = summarize(&allocation_counts);
    let (b_min, b_max, b_mean) = summarize(&bytes_allocated);
    let (d_min, d_max, d_mean) = summarize(&deallocations);

    println!(
        "\nprocess_stream_alloc: {} samples (one CP-mirror iteration each)\n  allocations: min={} max={} mean={:.1}\n  bytes_allocated (gross): min={} max={} mean={:.1}\n  deallocations: min={} max={} mean={:.1}",
        SAMPLES, a_min, a_max, a_mean, b_min, b_max, b_mean, d_min, d_max, d_mean
    );
}
