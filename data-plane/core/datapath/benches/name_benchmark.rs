// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use slim_datapath::api::{EncodedName, ProtoName, SlimHeader, StringName};
use slim_datapath::tables::SubscriptionTable;
use slim_datapath::tables::subscription_table::SubscriptionTableImpl;

fn make_proto_name() -> ProtoName {
    ProtoName {
        name: Some(EncodedName {
            component_0: 0x1234_5678_9abc_def0,
            component_1: 0xfeed_cafe_dead_beef,
            component_2: 0x0101_0101_0101_0101,
            component_3: u64::MAX,
        }),
        str_name: Some(StringName {
            str_component_0: "org".to_string(),
            str_component_1: "namespace".to_string(),
            str_component_2: "agent".to_string(),
        }),
    }
}

/// Build a SlimHeader whose destination matches the subscription used in routing benchmarks
/// (org/namespace/agent, id=42).
fn make_slim_header() -> SlimHeader {
    let proto_name = ProtoName::from_strings(["org", "namespace", "agent"]).with_id(42);
    SlimHeader {
        source: Some(proto_name.clone()),
        destination: Some(proto_name),
        identity: String::new(),
        fanout: 1,
        recv_from: None,
        forward_to: None,
        incoming_conn: None,
        error: None,
    }
}

fn bench_proto_name_from_strings(c: &mut Criterion) {
    c.bench_function("proto_name_from_strings", |b| {
        b.iter(|| {
            black_box(ProtoName::from_strings(black_box(["org", "namespace", "agent"])))
        })
    });
}

fn bench_proto_name_clone(c: &mut Criterion) {
    let name = ProtoName::from_strings(["org", "namespace", "agent"]);
    c.bench_function("proto_name_clone", |b| {
        b.iter(|| black_box(name.clone()))
    });
}

fn bench_proto_name_from_other(c: &mut Criterion) {
    let proto_name = make_proto_name();
    c.bench_function("proto_name_clone_from_ref", |b| {
        b.iter(|| black_box(proto_name.clone()))
    });
}

/// Routing flow via the EncodedName extracted from get_dst().
fn bench_routing_via_proto_name(c: &mut Criterion) {
    let table = SubscriptionTableImpl::default();
    let sub = ProtoName::from_strings(["org", "namespace", "agent"]).with_id(42);
    table.add_subscription(sub, 1, true, 1).unwrap();
    let slim_header = make_slim_header();

    c.bench_function("routing_via_proto_name", |b| {
        b.iter(|| {
            let dst = black_box(slim_header.get_dst());
            let enc = dst.name.as_ref().unwrap();
            black_box(table.match_one(black_box(enc), 0))
        })
    });
}

/// Optimized: full routing flow via the EncodedName path.
/// get_encoded_dst() is a Copy of 4 u64s — zero heap allocation.
fn bench_routing_via_encoded_name(c: &mut Criterion) {
    let table = SubscriptionTableImpl::default();
    let sub = ProtoName::from_strings(["org", "namespace", "agent"]).with_id(42);
    table.add_subscription(sub, 1, true, 1).unwrap();
    let slim_header = make_slim_header();

    c.bench_function("routing_via_encoded_name", |b| {
        b.iter(|| {
            let encoded = black_box(slim_header.get_encoded_dst());
            black_box(table.match_one(black_box(&encoded), 0))
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
