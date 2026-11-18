# agntcy-slim-persistence

Encrypted, disk-backed persistence for SLIM. Allows MLS group state and session
records to survive a process restart so sessions can rejoin without repeating the
invite/welcome handshake.

## Overview

Without persistence every process restart requires a full MLS invite/welcome
cycle. With it, `restore_sessions()` in the session layer reloads all session
state and issues a rejoin announcement automatically.

The design is deliberately split by target:

- **Native** — both a `SlimGroupStateStorage` (MLS group snapshots and epoch
  records) and a `SlimKvStore` (session records) are backed by a plain SQLite
  database. All stored values are encrypted with AES-256-GCM before being handed
  to the database; only keys (e.g. `session:<id>`) are plaintext.
- **`wasm32`** — there is no durable store. `SlimGroupStateStorage` falls back to
  the in-memory provider (state lives only for the page's lifetime). `SlimKvStore`
  is uninhabited on that target.

## Quick start

```rust
use slim_persistence::{PersistenceConfig, MlsEncryptionKey, PersistentStore};
use std::path::Path;

// Recommended: supply an explicit secret key.
let config = PersistenceConfig::with_key(
    "/var/lib/myapp/state",
    MlsEncryptionKey::Passphrase("my-secret-passphrase".into()),
);

// Open the unified store — one encrypted SQLite file for both MLS and session state.
let (group_storage, kv_store) =
    PersistentStore::open(&config.path, "alice", config.encryption_key)?;

// Pass group_storage to the MLS client builder.
// Pass kv_store to SessionLayer::new_with_persistence.

// Store a session record.
kv_store.put("session:42", &serialized_session_record)?;

// Retrieve it on the next start.
if let Some(bytes) = kv_store.get("session:42")? {
    // deserialize and rebuild the session
}

// List all sessions.
for (key, bytes) in kv_store.list_prefix("session:")? {
    // restore each session
}

// Clean up when a session is permanently closed.
kv_store.delete("session:42")?;
group_storage.delete_group(group_id)?;
```

## API

### `PersistenceConfig`

Serde-serializable configuration for the store. Can be embedded as a
`persistence:` section in YAML or JSON node configuration files.

```rust
// No explicit key: key is derived from the public app name.
// Provides tamper-detection but NOT confidentiality.
let config = PersistenceConfig::new("/var/lib/myapp/state");

// Explicit key: the only option that provides real confidentiality.
let config = PersistenceConfig::with_key(
    "/var/lib/myapp/state",
    MlsEncryptionKey::Passphrase("secret".into()),
);
```

### `MlsEncryptionKey`

Two variants for supplying the at-rest encryption key:

| Variant | Description |
|---|---|
| `Passphrase(String)` | A human-readable passphrase; a 256-bit key is derived via HKDF-SHA256. |
| `RawKey([u8; 32])` | Raw 32-byte AES-256 key material. |

`Debug` output is always redacted (`***`) — key material is never printed.

### `PersistentStore` (native only)

The primary entry point. Opens (or creates) a single SQLite file named
`slim-<hex(store_key)>.db` under the given directory and returns a
`SlimGroupStateStorage` and a `SlimKvStore` that share the same file and cipher.

```rust
let (group_storage, kv_store) = PersistentStore::open(
    Path::new("/var/lib/myapp/state"),
    "alice",          // stable identity key — also seeds the file name
    Some(encryption_key),
)?;
```

`store_key` must be stable across restarts for the same logical identity. It is
hex-encoded to form the database filename and is used to seed key derivation when
no explicit `MlsEncryptionKey` is provided.

### `SlimGroupStateStorage`

Implements `mls_rs_core::group::GroupStateStorage`. Pass the `Sqlite` variant to
the MLS client builder to enable durable group state.

```rust
// In-memory (default, no persistence):
let storage = SlimGroupStateStorage::in_memory();

// Durable SQLite (native only, standalone file):
let storage = SlimGroupStateStorage::open_sqlite(dir, "alice", Some(key))?;

storage.is_persistent(); // false for InMemory, true for Sqlite

// Remove a group's state on hard close:
storage.delete_group(group_id)?;

// List stored group IDs (native only):
let ids = storage.stored_groups()?;
```

### `SlimKvStore`

Encrypted key/value store for session-level state. Keys are plaintext; values are
AES-256-GCM encrypted.

```rust
kv_store.put("session:42", &bytes)?;
kv_store.get("session:42")?;                // → Option<Vec<u8>>
kv_store.list_prefix("session:")?;          // → Vec<(String, Vec<u8>)>
kv_store.delete("session:42")?;
```

### `PersistenceError`

```rust
pub enum PersistenceError {
    Io(std::io::Error),
    Storage(String),
}
```

Implements `mls_rs_core::error::IntoAnyError` so it can serve as the associated
`Error` type on `GroupStateStorage` implementations.

## Security

All values written to SQLite are encrypted with AES-256-GCM before being stored.
The wire format is `nonce (12 bytes) || ciphertext || AEAD tag`. Nonces are
randomly generated per write. Key derivation uses HKDF-SHA256 with
domain-separated salt and info constants. Encryption reuses the same `aws-lc-rs`
stack as the MLS layer — no OpenSSL or SQLCipher dependency is added.

**Key management:**

- Supply an explicit key via `MlsEncryptionKey` whenever the store may be read by
  an untrusted party. This is the only configuration that provides confidentiality.
- If no key is supplied, the key is derived from the public app name. This still
  provides tamper detection but **not confidentiality** — anyone who knows the app
  name can re-derive the key and read the store.

## Architecture

```
PersistenceConfig
    └── PersistentStore::open(dir, store_key, key?)
            │
            ├── SQLite WAL database  "slim-<hex(store_key)>.db"
            │       ├── group_state table
            │       └── application_data table
            │
            └── ValueCipher::derive(key?, store_key)  ← shared Arc
                    │                          │
                    ▼                          ▼
         SlimGroupStateStorage::Sqlite   SlimKvStore::Sqlite
         (mls-rs GroupStateStorage)      (put / get / list / delete)
         encrypt/decrypt on write/read   encrypt/decrypt on write/read
```

One cipher instance is shared between both handles — one key derivation, one
file, and one lifecycle cover all restorable state for an identity.

## Platform support

| Target | `SlimGroupStateStorage` | `SlimKvStore` | `PersistentStore` |
|---|---|---|---|
| Native (Linux, macOS, Windows) | `InMemory` or `Sqlite` | `Sqlite` | Available |
| `wasm32` | `InMemory` only | Uninhabited | Not available |

## License

Apache-2.0 — see [LICENSE](../../LICENSE).
