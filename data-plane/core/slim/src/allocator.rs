// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

/// When the `jemalloc` feature is enabled, replace the system allocator with
/// tikv-jemallocator (jemalloc). This benefits:
///
/// - **glibc targets** (`x86_64-unknown-linux-gnu`): improved throughput and
///   reduced fragmentation over glibc's ptmalloc2.
/// - **musl targets** (`x86_64-unknown-linux-musl`): jemalloc replaces musl's
///   minimal allocator, enabling performant static executables.
///
/// # Switching allocators
///
/// ```text
/// # Default system allocator
/// cargo build -p agntcy-slim
///
/// # jemalloc
/// cargo build -p agntcy-slim --features jemalloc
///
/// # jemalloc + musl static binary
/// cargo build -p agntcy-slim --features jemalloc --target x86_64-unknown-linux-musl
/// ```
///
/// # Benchmarking
///
/// ```text
/// # system allocator baseline
/// cargo bench -p agntcy-slim
///
/// # jemalloc
/// cargo bench -p agntcy-slim --features jemalloc
/// ```
#[cfg(feature = "jemalloc")]
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
