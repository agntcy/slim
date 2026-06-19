// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use slim_datapath::api::{NULL_COMPONENT, ProtoName, SlimHeader};
use slim_datapath::tables::SubscriptionTable;
use slim_datapath::tables::subscription_table::SubscriptionTableImpl;
use slim_datapath::tables::{ConnType, MatchFilter};

fn make_proto_name() -> ProtoName {
    ProtoName::from_strings(["org", "namespace", "agent"]).with_id(NULL_COMPONENT)
}

/// Build a SlimHeader whose destination matches the subscription used in routing benchmarks
/// (org/namespace/agent, id=42).
fn make_slim_header() -> SlimHeader {
    let proto_name = ProtoName::from_strings(["org", "namespace", "agent"]).with_id(42);
    SlimHeader::new(proto_name.clone(), proto_name, "", None)
}

fn bench_proto_name_from_strings(c: &mut Criterion) {
    c.bench_function("proto_name_from_strings", |b| {
        b.iter(|| {
            black_box(ProtoName::from_strings(black_box([
                "org",
                "namespace",
                "agent",
            ])))
        })
    });
}

fn bench_proto_name_clone(c: &mut Criterion) {
    let name = ProtoName::from_strings(["org", "namespace", "agent"]);
    c.bench_function("proto_name_clone", |b| b.iter(|| black_box(name.clone())));
}

fn bench_proto_name_from_other(c: &mut Criterion) {
    let proto_name = make_proto_name();
    c.bench_function("proto_name_clone_from_ref", |b| {
        b.iter(|| black_box(proto_name.clone()))
    });
}

/// Routing flow via components/id decoded from the destination Name.
fn bench_routing_via_proto_name(c: &mut Criterion) {
    let table = SubscriptionTableImpl::default();
    let sub = ProtoName::from_strings(["org", "namespace", "agent"]).with_id(42);
    table.add_subscription(sub, 1, ConnType::Local, 1).unwrap();
    let slim_header = make_slim_header();

    c.bench_function("routing_via_proto_name", |b| {
        b.iter(|| {
            let dst = black_box(slim_header.get_dst());
            black_box(table.match_one(
                black_box(dst.components()),
                black_box(dst.id()),
                0,
                MatchFilter::ALL,
            ))
        })
    });
}

/// Optimized: full routing flow via get_encoded_dst() — returns ([u64; 3], u128), zero alloc.
fn bench_routing_via_encoded_name(c: &mut Criterion) {
    let table = SubscriptionTableImpl::default();
    let sub = ProtoName::from_strings(["org", "namespace", "agent"]).with_id(42);
    table.add_subscription(sub, 1, ConnType::Local, 1).unwrap();
    let slim_header = make_slim_header();

    c.bench_function("routing_via_encoded_name", |b| {
        b.iter(|| {
            let (components, id) = black_box(slim_header.get_encoded_dst());
            black_box(table.match_one(black_box(components), black_box(id), 0, MatchFilter::ALL))
        })
    });
}

criterion_group!(
    benches,
    bench_proto_name_from_strings,
    bench_proto_name_clone,
    bench_proto_name_from_other,
    bench_routing_via_proto_name,
    bench_routing_via_encoded_name,
);

criterion_main!(benches);
