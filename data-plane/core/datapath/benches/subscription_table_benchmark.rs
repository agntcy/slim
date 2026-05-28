// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use slim_datapath::api::{EncodedName, ProtoName};
use slim_datapath::tables::SubscriptionTable;
use slim_datapath::tables::subscription_table::SubscriptionTableImpl;
use slim_datapath::tables::{ConnCategory, MatchFilter};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn encoded(name: &ProtoName) -> EncodedName {
    name.name.unwrap()
}

/// Build a table with `n` distinct specific IDs under one prefix.
/// The target is the last-inserted ID — worst-case scan position.
fn build_specific_ids(n: usize) -> (SubscriptionTableImpl, EncodedName) {
    let base = ProtoName::from_strings(["org", "ns", "svc"]);
    let table = SubscriptionTableImpl::default();
    for i in 1..=n {
        let name = base.clone().with_id(i as u64);
        table
            .add_subscription(name, (i + 100) as u64, ConnCategory::Remote, i as u64)
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
    table
        .add_subscription(base.clone(), 1, ConnCategory::Remote, 1)
        .unwrap();
    for i in 1..n {
        let name = base.clone().with_id(i as u64);
        table
            .add_subscription(
                name,
                (i + 100) as u64,
                ConnCategory::Remote,
                (i + 100) as u64,
            )
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
        let category = if i < half {
            ConnCategory::Local
        } else {
            ConnCategory::Remote
        };
        table
            .add_subscription(name.clone(), (i + 10) as u64, category, i as u64)
            .unwrap();
    }
    (table, encoded(&name))
}

/// Build a table with `n` distinct prefixes (org/ns/svcN), each with 1
/// specific ID (id=1) and 1 remote connection.  The returned target is the
/// last-inserted prefix — every lookup costs exactly one HashMap probe
/// regardless of table size, so this group isolates HashMap / cache-miss
/// overhead as the routing table grows.
fn build_many_prefixes(n: usize) -> (SubscriptionTableImpl, EncodedName) {
    let table = SubscriptionTableImpl::default();
    for i in 0..n {
        let svc = format!("svc{i}");
        let name = ProtoName::from_strings(["org", "ns", svc.as_str()]).with_id(1);
        table
            .add_subscription(name, (i + 100) as u64, ConnCategory::Remote, i as u64)
            .unwrap();
    }
    let last_svc = format!("svc{}", n - 1);
    let target = ProtoName::from_strings(["org", "ns", last_svc.as_str()]).with_id(1);
    (table, encoded(&target))
}

// ── match_one benchmarks ──────────────────────────────────────────────────────

/// Deep / single-prefix topology: `n` distinct IDs under one prefix.
/// Target is last (worst-case linear scan over by_id).
fn bench_match_one_specific_id(c: &mut Criterion) {
    let mut group = c.benchmark_group("match_one/specific_id");
    for &n in &[1usize, 4, 8, 16, 64, 256, 1024] {
        let (table, enc) = build_specific_ids(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &enc, |b, e| {
            b.iter(|| black_box(table.match_one(black_box(e), u64::MAX, MatchFilter::ALL)))
        });
    }
    group.finish();
}

/// Deep / single-prefix topology: NULL_COMPONENT over `n` IdEntries.
fn bench_match_one_null_component(c: &mut Criterion) {
    let mut group = c.benchmark_group("match_one/null_component");
    for &n in &[1usize, 4, 8, 16, 64, 256] {
        let (table, enc) = build_with_null(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &enc, |b, e| {
            b.iter(|| black_box(table.match_one(black_box(e), u64::MAX, MatchFilter::ALL)))
        });
    }
    group.finish();
}

/// Local-preference fast path: single IdEntry, locals preferred over remotes.
fn bench_match_one_local_preference(c: &mut Criterion) {
    let name = ProtoName::from_strings(["org", "ns", "svc"]);
    let table = SubscriptionTableImpl::default();
    // 4 remote connections
    for i in 0u64..4 {
        table
            .add_subscription(name.clone(), i, ConnCategory::Remote, i)
            .unwrap();
    }
    // 2 local connections (should be preferred)
    for i in 10u64..12 {
        table
            .add_subscription(name.clone(), i, ConnCategory::Local, i)
            .unwrap();
    }
    let enc = encoded(&name);
    c.bench_function("match_one/local_preference", |b| {
        b.iter(|| black_box(table.match_one(black_box(&enc), u64::MAX, MatchFilter::ALL)))
    });
}

/// Wide topology: `n` distinct prefixes in the routing table, each with
/// 1 ID and 1 connection.  One specific prefix is targeted on every call.
/// Per-prefix work is O(1); the variable is HashMap / cache-miss cost.
fn bench_match_one_many_prefixes(c: &mut Criterion) {
    let mut group = c.benchmark_group("match_one/many_prefixes");
    for &n in &[64usize, 256, 1024, 4096] {
        let (table, enc) = build_many_prefixes(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &enc, |b, e| {
            b.iter(|| black_box(table.match_one(black_box(e), u64::MAX, MatchFilter::ALL)))
        });
    }
    group.finish();
}

// ── match_all benchmarks ──────────────────────────────────────────────────────

/// Deep / single-prefix topology: one ID with `n` connections (half local).
fn bench_match_all_specific_id(c: &mut Criterion) {
    let mut group = c.benchmark_group("match_all/specific_id");
    for &n in &[1usize, 4, 8, 16, 64, 256] {
        let (table, enc) = build_one_id_n_conns(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &enc, |b, e| {
            b.iter(|| black_box(table.match_all(black_box(e), u64::MAX, MatchFilter::ALL)))
        });
    }
    group.finish();
}

/// Deep / single-prefix topology: NULL_COMPONENT union across `n` IdEntries.
fn bench_match_all_null_component(c: &mut Criterion) {
    let mut group = c.benchmark_group("match_all/null_component");
    for &n in &[1usize, 4, 8, 16, 64, 256] {
        let (table, enc) = build_with_null(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &enc, |b, e| {
            b.iter(|| black_box(table.match_all(black_box(e), u64::MAX, MatchFilter::ALL)))
        });
    }
    group.finish();
}

/// Wide topology: `n` distinct prefixes, one targeted per call.
/// Complements match_one/many_prefixes via the match_all code path.
fn bench_match_all_many_prefixes(c: &mut Criterion) {
    let mut group = c.benchmark_group("match_all/many_prefixes");
    for &n in &[64usize, 256, 1024, 4096] {
        let (table, enc) = build_many_prefixes(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &enc, |b, e| {
            b.iter(|| black_box(table.match_all(black_box(e), u64::MAX, MatchFilter::ALL)))
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_match_one_specific_id,
    bench_match_one_null_component,
    bench_match_one_local_preference,
    bench_match_one_many_prefixes,
    bench_match_all_specific_id,
    bench_match_all_null_component,
    bench_match_all_many_prefixes,
);
criterion_main!(benches);
