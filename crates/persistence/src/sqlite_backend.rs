// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Shared native SQLite plumbing for the disk-backed stores.
//!
//! The database is plain (unencrypted) SQLite — statically bundled, so nothing
//! is linked against a system `libsqlite3`. Confidentiality/integrity of the
//! stored blobs is provided at the value level with `aws-lc-rs` (see
//! [`crate::cipher`]), which avoids linking OpenSSL/SQLCipher. Native only;
//! there is no SQLite on `wasm32`.

use std::path::Path;

use mls_rs_provider_sqlite::{
    JournalMode, SqLiteDataStorageEngine, connection_strategy::FileConnectionStrategy,
};

use crate::errors::PersistenceError;

/// The concrete plain-file storage engine type used across the crate.
pub(crate) type Engine = SqLiteDataStorageEngine<FileConnectionStrategy>;

/// Open a plain SQLite engine at `db_path` (created if absent). `journal_mode`
/// tunes SQLite's journaling (e.g. `Wal` for better read/write concurrency).
pub(crate) fn open_engine(
    db_path: &Path,
    journal_mode: Option<JournalMode>,
) -> Result<Engine, PersistenceError> {
    let engine = SqLiteDataStorageEngine::new(FileConnectionStrategy::new(db_path))
        .map_err(|e| PersistenceError::Storage(e.to_string()))?
        .with_journal_mode(journal_mode);
    Ok(engine)
}
