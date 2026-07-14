// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Micro-benchmarks for `SharedSecret` token issuance and verification.
//!
//! Run with:
//! ```text
//! cargo bench -p agntcy-slim-auth --bench shared_secret_benchmark
//! ```
//!
//! Criterion reports `time` per iteration; throughput is reported as
//! elements/sec so you can read the result directly as "tokens per second".

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use slim_auth::shared_secret::SharedSecret;
use slim_auth::traits::{TokenProvider, Verifier};

const SECRET: &str = "abcdefghijklmnopqrstuvwxyz012345"; // 32 bytes, satisfies MIN_SECRET_LEN

fn make_auth() -> SharedSecret {
    SharedSecret::new("svc", SECRET).expect("valid shared secret")
}

fn make_auth_replay() -> SharedSecret {
    SharedSecret::builder("svc", SECRET)
        .replay_cache(1 << 16)
        .build()
        .expect("valid shared secret")
}

fn bench_get_token(c: &mut Criterion) {
    let auth = make_auth();
    let mut group = c.benchmark_group("shared_secret/get_token");
    group.throughput(Throughput::Elements(1));
    group.bench_function("default", |b| {
        b.iter(|| {
            let t = auth.get_token().expect("token");
            black_box(t);
        });
    });
    group.bench_function("into_buf", |b| {
        // Reuse a single buffer across iterations; measures the steady-state cost
        // when the caller owns the output String.
        let mut buf = String::with_capacity(160);
        b.iter(|| {
            auth.get_token_into(&mut buf).expect("token");
            black_box(buf.as_str());
        });
    });
    group.finish();
}

fn bench_try_verify(c: &mut Criterion) {
    let auth = make_auth(); // replay disabled => same token can be verified repeatedly
    let token = auth.get_token().expect("token");
    let mut group = c.benchmark_group("shared_secret/try_verify");
    group.throughput(Throughput::Elements(1));
    group.bench_function("default", |b| {
        // Passes &str directly — no per-iteration String allocation.
        b.iter(|| {
            auth.try_verify(black_box(token.as_str())).expect("verify");
        });
    });
    group.finish();
}

fn bench_round_trip(c: &mut Criterion) {
    // End-to-end issue + verify in the same iteration.
    // Use replay-enabled auth to also exercise the replay cache insert path.
    let auth = make_auth_replay();
    let mut group = c.benchmark_group("shared_secret/issue_and_verify");
    group.throughput(Throughput::Elements(1));
    group.bench_function("replay_enabled", |b| {
        b.iter(|| {
            let token = auth.get_token().expect("token");
            auth.try_verify(black_box(token)).expect("verify");
        });
    });
    group.finish();
}

criterion_group!(benches, bench_get_token, bench_try_verify, bench_round_trip);
criterion_main!(benches);
