// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use slim_datapath::api::{EncodedName, ProtoName};
use slim_datapath::tables::SubscriptionTable;
use slim_datapath::tables::subscription_table::SubscriptionTableImpl;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn encoded(name: &ProtoName) -> EncodedName {
    name.name.unwrap()
}

/// Build a table with `n` distinct specific IDs under one prefix.
/// The target returned is the last-inserted ID — worst-case scan position.
fn build_specific_ids(n: usize) -> (SubscriptionTableImpl, EncodedName) {
    let base = ProtoName::from_strings(["org", "ns", "svc"]);
    let table = SubscriptionTableImpl::default();
    for i in 1..=n {
        let name = base.clone().with_id(i as u64);
        table
            .add_subscription(name, (i + 100) as u64, false, i as u64)
            .unwrap();
    }
    let target = base.with_id(n as u64);
    (table, encoded(&target))
}

/// Build a table with `n` entries under one prefix: 1 NULL_COMPONENT entry
/// (conn 1) plus n−1 specific IDs, each with 1 remote connection.
/// Returns the encoded NULL_COMPONENT name for fan-out benchmarks.
fn build_with_null(n: usize) -> (SubscriptionTableImpl, EncodedName) {
    let base = ProtoName::from_strings(["org", "ns", "svc"]);
    let table = SubscriptionTableImpl::default();
    table.add_subscription(base.clone(), 1, false, 1).unwrap();
    for i in 1..n {
        let name = base.clone().with_id(i as u64);
        table
            .add_subscription(name, (i + 100) as u64, false, (i + 100) as u64)
            .unwrap();
    }
    (table, encoded(&base))
}

/// Build a table with a single specific ID (42) having `n_conns` connections.
/// The first half are local, the rest remote.
fn build_one_id_n_conns(n_conns: usize) -> (SubscriptionTableImpl, EncodedName) {
    let name = ProtoName::from_strings(["org", "ns", "svc"]).with_id(42);
    let table = SubscriptionTableImpl::default();
    let half = n_conns.div_ceil(2);
    for i in 0..n_conns {
        let is_local = i < half;
        table
            .add_subscription(name.clone(), (i + 10) as u64, is_local, i as u64)
            .unwrap();
    }
    (table, encoded(&name))
}

// ── match_one benchmarks ──────────────────────────────────────────────────────

/// Linear scan over `by_id` to find a specific component_3.
/// `n` = number of distinct IDs in the prefix; target is last (worst case).
fn bench_match_one_specific_id(c: &mut Criterion) {
    let mut group = c.benchmark_group("match_one/specific_id");
    for &n in &[1usize, 4, 8, 16] {
        let (table, enc) = build_specific_ids(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &enc, |b, e| {
            b.iter(|| black_box(table.match_one(black_box(e), u64::MAX)))
        });
    }
    group.finish();
}

/// NULL_COMPONENT fan-out: aggregate unique locals/remotes across `n` IdEntries.
/// `n` = total IdEntries (1 NULL_COMPONENT + n−1 specific IDs).
fn bench_match_one_null_component(c: &mut Criterion) {
    let mut group = c.benchmark_group("match_one/null_component");
    for &n in &[1usize, 4, 8, 16] {
        let (table, enc) = build_with_null(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &enc, |b, e| {
            b.iter(|| black_box(table.match_one(black_box(e), u64::MAX)))
        });
    }
    group.finish();
}

/// match_one with both local and remote connections present — verifies the
/// local-preference fast path returns without scanning remotes.
fn bench_match_one_local_preference(c: &mut Criterion) {
    let name = ProtoName::from_strings(["org", "ns", "svc"]);
    let table = SubscriptionTableImpl::default();
    // 4 remote connections
    for i in 0u64..4 {
        table
            .add_subscription(name.clone(), i, false, i)
            .unwrap();
    }
    // 2 local connections (should be preferred)
    for i in 10u64..12 {
        table
            .add_subscription(name.clone(), i, true, i)
            .unwrap();
    }
    let enc = encoded(&name);
    c.bench_function("match_one/local_preference", |b| {
        b.iter(|| black_box(table.match_one(black_box(&enc), u64::MAX)))
    });
}

// ── match_all benchmarks ──────────────────────────────────────────────────────

/// Collect all connections for a specific ID.
/// `n` = number of connections (first half local, rest remote).
fn bench_match_all_specific_id(c: &mut Criterion) {
    let mut group = c.benchmark_group("match_all/specific_id");
    for &n in &[1usize, 4, 8, 16] {
        let (table, enc) = build_one_id_n_conns(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &enc, |b, e| {
            b.iter(|| black_box(table.match_all(black_box(e), u64::MAX)))
        });
    }
    group.finish();
}

/// NULL_COMPONENT union: collect unique connections across `n` IdEntries.
/// `n` = total IdEntries (1 NULL_COMPONENT + n−1 specific IDs, 1 conn each).
fn bench_match_all_null_component(c: &mut Criterion) {
    let mut group = c.benchmark_group("match_all/null_component");
    for &n in &[1usize, 4, 8, 16] {
        let (table, enc) = build_with_null(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &enc, |b, e| {
            b.iter(|| black_box(table.match_all(black_box(e), u64::MAX)))
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_match_one_specific_id,
    bench_match_one_null_component,
    bench_match_one_local_preference,
    bench_match_all_specific_id,
    bench_match_all_null_component,
);
criterion_main!(benches);
