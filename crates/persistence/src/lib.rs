// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Encrypted, disk-backed persistence for SLIM.
//!
//! Provides the storage primitives used to restore sessions after a restart:
//! an MLS group-state store (implementing `mls-rs`' `GroupStateStorage`), a
//! blob key/value store for SLIM session records, and [`PersistentStore`] which
//! unifies both over a single database file per identity.
//!
//! Native targets use a plain, statically-bundled SQLite database whose stored
//! values are encrypted with AES-256-GCM via `aws-lc-rs` (the same crypto stack
//! the MLS layer uses — no OpenSSL or SQLCipher is linked).
//!
//! **Durable persistence is native-only.** On `wasm32` a page reload discards
//! the WebAssembly instance's memory, so there is nothing durable to restore;
//! persistence is therefore not offered there ([`PersistentStore`] is
//! native-only and the session layer treats persistence as disabled). The
//! in-memory [`SlimGroupStateStorage`] still exists on wasm as the MLS client's
//! required in-session (non-persisted) storage.

pub mod errors;
pub mod group_storage;
pub mod kv_store;
pub mod store;

// Native-only plumbing: plain SQLite engine + AES-256-GCM value encryption.
#[cfg(not(target_arch = "wasm32"))]
mod cipher;
#[cfg(not(target_arch = "wasm32"))]
mod sqlite_backend;

pub use errors::PersistenceError;
pub use group_storage::{MlsEncryptionKey, SlimGroupStateStorage};
pub use kv_store::SlimKvStore;
pub use store::PersistenceConfig;

// Durable persistence is native-only (needs a filesystem). On `wasm32` a page
// reload discards the WebAssembly instance's memory, so there is nothing to
// restore; persistence is treated as disabled there.
#[cfg(not(target_arch = "wasm32"))]
pub use store::PersistentStore;

/// Fuzzing-only surface for `crates/persistence/fuzz`
/// (`agntcy-slim-persistence-fuzz`).
///
/// `ValueCipher` and its `encrypt`/`decrypt` are `pub(crate)` in [`cipher`]
/// because at-rest encryption is an internal detail, not public API. This
/// module exists only behind the `fuzzing` feature (off by default, not part
/// of any default feature set) so the fuzz crate can drive real
/// encrypt/decrypt round trips without making the cipher generally `pub`;
/// `#[doc(hidden)]` also keeps it out of rustdoc for anyone who does enable
/// the feature. Not covered by semver.
#[cfg(all(feature = "fuzzing", not(target_arch = "wasm32")))]
#[doc(hidden)]
pub mod fuzzing {
    pub use crate::cipher::decrypt_fuzz as decrypt;
    pub use crate::cipher::encrypt_fuzz as encrypt;
}
