// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use slim_auth::shared_secret::SharedSecret;
use slim_datapath::api::ProtoMessage as Message;
use slim_mls::mls::Mls;
use slim_session::MessageDirection;
use slim_session::mls_state::MlsState;
use slim_testing::utils::TEST_VALID_SECRET;

fn setup_session(percent: u32) -> (MlsState<SharedSecret, SharedSecret>, Message) {
    let mut alice_mls = Mls::new(
        SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
        SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
    );
    let mut bob_mls = Mls::new(
        SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
        SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
    );

    alice_mls.initialize().unwrap();
    bob_mls.initialize().unwrap();

    let _group_id = alice_mls.create_group().unwrap();
    let bob_key_package = bob_mls.generate_key_package().unwrap();
    let ret = alice_mls.add_member(&bob_key_package).unwrap();
    bob_mls.process_welcome(&ret.welcome_message).unwrap();

    let mut alice_state = MlsState::new(alice_mls, percent).unwrap();
    let bob_state = MlsState::new(bob_mls, percent).unwrap();

    let original_payload = b"Hello Bob! This is a benchmark message with some realistic size.";

    let mut alice_msg = Message::builder()
        .source(slim_datapath::api::ProtoName::from_strings(["org", "default", "alice"]).with_id(0))
        .destination(slim_datapath::api::ProtoName::from_strings([
            "org", "default", "bob",
        ]))
        .session_id(1)
        .message_id(1)
        .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
        .session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg)
        .application_payload("text", original_payload.to_vec())
        .build_publish()
        .unwrap();

    alice_state
        .process_message(&mut alice_msg, MessageDirection::South)
        .unwrap();

    (bob_state, alice_msg)
}

fn bench_header_integrity(c: &mut Criterion) {
    let mut group = c.benchmark_group("header_integrity_validation");

    group.bench_function("0_percent_skipped", |b| {
        b.iter_batched(
            || setup_session(0),
            |(mut bob_state, mut alice_msg)| {
                bob_state
                    .process_message(&mut alice_msg, MessageDirection::North)
                    .unwrap();
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("50_percent_stochastic", |b| {
        b.iter_batched(
            || setup_session(50),
            |(mut bob_state, mut alice_msg)| {
                bob_state
                    .process_message(&mut alice_msg, MessageDirection::North)
                    .unwrap();
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("100_percent_strict", |b| {
        b.iter_batched(
            || setup_session(100),
            |(mut bob_state, mut alice_msg)| {
                bob_state
                    .process_message(&mut alice_msg, MessageDirection::North)
                    .unwrap();
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_header_integrity);
criterion_main!(benches);
