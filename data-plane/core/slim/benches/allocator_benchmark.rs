// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Allocator performance benchmarks.
//!
//! Run with the system allocator (baseline):
//!
//! ```text
//! cargo bench -p agntcy-slim
//! ```
//!
//! Run with jemalloc for comparison:
//!
//! ```text
//! cargo bench -p agntcy-slim --features jemalloc
//! ```
//!
//! The active allocator is printed as part of the benchmark group name so that
//! reports from both runs can be compared side-by-side.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};

fn allocator_label() -> &'static str {
    if cfg!(feature = "jemalloc") {
        "jemalloc"
    } else {
        "system"
    }
}

// ---------------------------------------------------------------------------
// Small allocations (64 B)
// ---------------------------------------------------------------------------

fn bench_small(c: &mut Criterion) {
    const SIZE: usize = 64;
    let mut group = c.benchmark_group(format!("[{}] small alloc", allocator_label()));

    group.bench_function(BenchmarkId::new("Vec::with_capacity", SIZE), |b| {
        b.iter(|| {
            let v: Vec<u8> = black_box(Vec::with_capacity(SIZE));
            drop(black_box(v));
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Medium allocations (4 KiB)
// ---------------------------------------------------------------------------

fn bench_medium(c: &mut Criterion) {
    const SIZE: usize = 4096;
    let mut group = c.benchmark_group(format!("[{}] medium alloc", allocator_label()));

    group.bench_function(BenchmarkId::new("Vec::with_capacity", SIZE), |b| {
        b.iter(|| {
            let v: Vec<u8> = black_box(Vec::with_capacity(SIZE));
            drop(black_box(v));
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Large allocations (1 MiB)
// ---------------------------------------------------------------------------

fn bench_large(c: &mut Criterion) {
    const SIZE: usize = 1_048_576; // 1 MiB
    let mut group = c.benchmark_group(format!("[{}] large alloc", allocator_label()));

    group.bench_function(BenchmarkId::new("Vec::with_capacity", SIZE), |b| {
        b.iter(|| {
            let v: Vec<u8> = black_box(Vec::with_capacity(SIZE));
            drop(black_box(v));
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Mixed sizes (16 B – 16 KiB, round-robin)
// ---------------------------------------------------------------------------

fn bench_mixed(c: &mut Criterion) {
    const SIZES: [usize; 6] = [16, 64, 256, 1024, 4096, 16384];
    let mut group = c.benchmark_group(format!("[{}] mixed alloc", allocator_label()));

    for size in SIZES {
        group.bench_function(BenchmarkId::new("Vec::with_capacity", size), |b| {
            b.iter(|| {
                let v: Vec<u8> = black_box(Vec::with_capacity(size));
                drop(black_box(v));
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Fragmentation stress: hold N live allocations before dropping all at once.
// Exercises arena / free-list management under memory pressure.
// ---------------------------------------------------------------------------

fn bench_fragmentation(c: &mut Criterion) {
    const N: usize = 10_000;
    const SIZES: [usize; 5] = [32, 128, 512, 2048, 8192];
    let mut group = c.benchmark_group(format!("[{}] fragmentation", allocator_label()));

    group.bench_function(
        BenchmarkId::new("alloc+drop N live blocks", N),
        |b| {
            b.iter(|| {
                let blocks: Vec<Vec<u8>> = (0..N)
                    .map(|i| black_box(Vec::with_capacity(SIZES[i % SIZES.len()])))
                    .collect();
                drop(black_box(blocks));
            });
        },
    );

    group.finish();
}

criterion_group!(
    benches,
    bench_small,
    bench_medium,
    bench_large,
    bench_mixed,
    bench_fragmentation,
);
criterion_main!(benches);
