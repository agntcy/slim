// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Identity providers and verifiers implemented in the foreign language.
//!
//! The configuration enums in [`crate::identity_config`] only select one of the
//! built-in mechanisms. The traits here let the host language supply its own:
//! implement [`IdentityProvider`] and/or [`IdentityVerifier`] and pass the
//! objects to [`App::new_with_custom_identity`](crate::App).
//!
//! # What a custom identity must do
//!
//! SLIM signs session control messages with the provider's signature key pair, and the
//! peer checks the signature with the key its verifier returns from `get_public_key`:
//!
//! - the provider holds a key pair ([`generate_signature_keys`] makes one), returns it
//!   from `get_signature_keys`, and swaps it on `set_signature_keys`;
//! - the provider's tokens let the paired verifier recover and trust that key: a claim, a
//!   DPoP confirmation, anything your scheme uses. The verifier returns the key from
//!   `get_public_key`, and `verify_key_binding` checks that a token is bound to a
//!   given key (MLS uses that one).
//!
//! # Example (Rust)
//!
//! A toy scheme, implemented against the same traits a foreign language would
//! implement. Its token is `<subject>.<base64 public key>.<secret>`, which is not
//! secure: a real provider signs its tokens (JWT, DPoP, ...) and a real verifier
//! checks that signature.
//!
//! ```rust,no_run
//! use std::sync::Arc;
//!
//! use parking_lot::Mutex;
//! use slim_bindings::{
//!     App, Direction, IdentityCallbackError, IdentityExport, IdentityProvider,
//!     IdentityVerifier, Name, SignatureKeys, generate_signature_keys,
//! };
//!
//! fn b64(bytes: &[u8]) -> String {
//!     use base64::Engine as _;
//!     base64::engine::general_purpose::STANDARD.encode(bytes)
//! }
//!
//! struct ToyProvider {
//!     id: String,
//!     secret: String,
//!     keys: Mutex<SignatureKeys>,
//!     installed: Mutex<bool>,
//! }
//!
//! #[async_trait::async_trait]
//! impl IdentityProvider for ToyProvider {
//!     async fn initialize(&self) -> Result<(), IdentityCallbackError> {
//!         Ok(())
//!     }
//!     fn get_token(&self) -> Result<String, IdentityCallbackError> {
//!         // Bind the current public key: it changes if SLIM installs a new pair.
//!         let public_key = b64(&self.keys.lock().public_key);
//!         Ok(format!("{}.{}.{}", self.id, public_key, self.secret))
//!     }
//!     fn get_id(&self) -> Result<String, IdentityCallbackError> {
//!         Ok(self.id.clone())
//!     }
//!     fn get_signature_keys(&self) -> Result<SignatureKeys, IdentityCallbackError> {
//!         Ok(self.keys.lock().clone())
//!     }
//!     fn mls_signature_keys_installed(&self) -> Result<bool, IdentityCallbackError> {
//!         Ok(*self.installed.lock())
//!     }
//!     fn set_signature_keys(
//!         &self,
//!         secret_key: Vec<u8>,
//!         public_key: Vec<u8>,
//!     ) -> Result<(), IdentityCallbackError> {
//!         *self.keys.lock() = SignatureKeys { secret_key, public_key };
//!         *self.installed.lock() = true;
//!         Ok(())
//!     }
//!     // No persistence.
//!     fn export_identity(&self) -> Result<Option<IdentityExport>, IdentityCallbackError> {
//!         Ok(None)
//!     }
//!     fn holds_long_term_signing_key(&self) -> Result<bool, IdentityCallbackError> {
//!         Ok(false)
//!     }
//!     fn restore_identity(&self, _: IdentityExport) -> Result<(), IdentityCallbackError> {
//!         Ok(())
//!     }
//! }
//!
//! struct ToyVerifier {
//!     secret: String,
//! }
//!
//! impl ToyVerifier {
//!     /// Returns `(subject, base64 public key)` of a valid token.
//!     fn parse(&self, token: &str) -> Result<(String, String), IdentityCallbackError> {
//!         match token.splitn(3, '.').collect::<Vec<_>>()[..] {
//!             [subject, public_key, secret] if secret == self.secret => {
//!                 Ok((subject.into(), public_key.into()))
//!             }
//!             _ => Err(IdentityCallbackError::Failed { message: "invalid token".into() }),
//!         }
//!     }
//! }
//!
//! #[async_trait::async_trait]
//! impl IdentityVerifier for ToyVerifier {
//!     async fn initialize(&self) -> Result<(), IdentityCallbackError> {
//!         Ok(())
//!     }
//!     async fn verify_async(&self, token: String) -> Result<(), IdentityCallbackError> {
//!         self.verify(token)
//!     }
//!     fn verify(&self, token: String) -> Result<(), IdentityCallbackError> {
//!         self.parse(&token).map(|_| ())
//!     }
//!     fn get_claims(&self, token: String) -> Result<String, IdentityCallbackError> {
//!         let (subject, _) = self.parse(&token)?;
//!         Ok(format!(r#"{{"sub":"{subject}"}}"#))
//!     }
//!     fn get_public_key(&self, token: String) -> Result<Vec<u8>, IdentityCallbackError> {
//!         use base64::Engine as _;
//!         base64::engine::general_purpose::STANDARD
//!             .decode(self.parse(&token)?.1)
//!             .map_err(|e| IdentityCallbackError::Failed { message: e.to_string() })
//!     }
//!     fn get_subject(&self, token: String) -> Result<String, IdentityCallbackError> {
//!         Ok(self.parse(&token)?.0)
//!     }
//!     fn verify_key_binding(
//!         &self,
//!         token: String,
//!         public_key: Vec<u8>,
//!     ) -> Result<(), IdentityCallbackError> {
//!         if self.get_public_key(token)? == public_key {
//!             Ok(())
//!         } else {
//!             Err(IdentityCallbackError::PublicKeyMismatch {
//!                 message: "token bound to another key".into(),
//!             })
//!         }
//!     }
//! }
//!
//! # async fn demo() -> Result<(), slim_bindings::SlimError> {
//! let provider = Arc::new(ToyProvider {
//!     id: "org/app/v1".into(),
//!     secret: "s3cret".into(),
//!     keys: Mutex::new(generate_signature_keys()?),
//!     installed: Mutex::new(false),
//! });
//! let verifier = Arc::new(ToyVerifier { secret: "s3cret".into() });
//!
//! let name = Arc::new(Name::new("org".into(), "app".into(), "v1".into()));
//! let app = App::new_with_custom_identity_async(name, provider, verifier, Direction::Bidirectional)
//!     .await?;
//! # Ok(()) }
//! ```
//!
//! # Example (Python)
//!
//! Python needs the event loop registered, because SLIM awaits the async methods from
//! its own threads, and the async constructor, because the blocking one would wait for
//! `initialize` on the loop it is blocking:
//!
//! ```python
//! class ToyProvider(slim_bindings.IdentityProvider):
//!     def __init__(self, id, secret):
//!         self.id, self.secret = id, secret
//!         self.keys = slim_bindings.generate_signature_keys()
//!         self.installed = False
//!
//!     async def initialize(self): ...
//!     def get_id(self): return self.id
//!     def get_token(self):
//!         public_key = base64.b64encode(self.keys.public_key).decode()
//!         return f"{self.id}.{public_key}.{self.secret}"
//!     def get_signature_keys(self): return self.keys
//!     def mls_signature_keys_installed(self): return self.installed
//!     def set_signature_keys(self, private_key, public_key):
//!         self.keys = slim_bindings.SignatureKeys(secret_key=private_key, public_key=public_key)
//!         self.installed = True
//!     def export_identity(self): return None
//!     def holds_long_term_signing_key(self): return False
//!     def restore_identity(self, identity): ...
//!
//! class ToyVerifier(slim_bindings.IdentityVerifier):
//!     def __init__(self, secret): self.secret = secret
//!     def parse(self, token):
//!         subject, public_key, secret = token.split(".", 2)
//!         if secret != self.secret:
//!             raise slim_bindings.IdentityCallbackError.Failed("invalid token")
//!         return subject, base64.b64decode(public_key)
//!
//!     async def initialize(self): ...
//!     async def verify_async(self, token): self.parse(token)
//!     def verify(self, token): self.parse(token)
//!     def get_claims(self, token): return json.dumps({"sub": self.parse(token)[0]})
//!     def get_subject(self, token): return self.parse(token)[0]
//!     def get_public_key(self, token): return self.parse(token)[1]
//!     def verify_key_binding(self, token, public_key):
//!         if self.parse(token)[1] != public_key:
//!             raise slim_bindings.IdentityCallbackError.PublicKeyMismatch("another key")
//!
//! slim_bindings.uniffi_set_event_loop(asyncio.get_running_loop())
//! app = await slim_bindings.App.new_with_custom_identity_async(
//!     name, ToyProvider(str(name), secret), ToyVerifier(secret),
//!     slim_bindings.Direction.BIDIRECTIONAL)
//! ```
//!

use std::sync::Arc;

use serde::de::DeserializeOwned;
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::errors::AuthError;
use slim_auth::identity_claims::IdentityClaims;
use slim_auth::traits::{ExportedIdentity, TokenProvider, Verifier};

use crate::errors::SlimError;

/// Error returned by foreign [`IdentityProvider`] / [`IdentityVerifier`] implementations.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum IdentityCallbackError {
    /// Generic failure (invalid token, signing error, ...).
    #[error("{message}")]
    Failed { message: String },
    /// Only for [`IdentityVerifier::verify_key_binding`]: the token is valid but
    /// bound to a different key than the one given.
    #[error("public key mismatch: {message}")]
    PublicKeyMismatch { message: String },
    /// Only for [`IdentityProvider::restore_identity`]: the persisted identity does not
    /// belong to this provider (for instance it was issued for another key or subject).
    /// SLIM then turns persistence off for the app, instead of resuming MLS state that
    /// belongs to someone else. Any other error keeps the provider's current identity and
    /// carries on with the persisted state.
    #[error("persisted identity does not match: {message}")]
    IdentityMismatch { message: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for IdentityCallbackError {
    fn from(err: uniffi::UnexpectedUniFFICallbackError) -> Self {
        IdentityCallbackError::Failed {
            message: err.reason,
        }
    }
}

impl From<IdentityCallbackError> for AuthError {
    fn from(err: IdentityCallbackError) -> Self {
        match err {
            IdentityCallbackError::Failed { message } => AuthError::Custom(message),
            IdentityCallbackError::PublicKeyMismatch { message } => AuthError::PublicKeyMismatch {
                subject: String::new(),
                expected: String::new(),
                found: message,
            },
            IdentityCallbackError::IdentityMismatch { message } => {
                AuthError::IdentityMismatch(message)
            }
        }
    }
}

/// The MLS signature key pair of an identity.
///
/// # Key format
///
/// The keys are opaque bytes in the encoding of the MLS crypto provider (AWS-LC),
/// which depends on the ciphersuite the library was built with. The default is P-256;
/// building with `insecure-self-certified-did` switches the whole build to Ed25519, so do not assume one format without knowing how your
/// SLIM library was built:
///
/// | Build | `secret_key` | `public_key` |
/// |-------|--------------|--------------|
/// | default (P-256, ECDSA) | at most 32 bytes: the raw big-endian private scalar, see below | 65 bytes: uncompressed SEC1 point: the byte `0x04`, then X, then Y |
/// | `insecure-self-certified-did` (Ed25519) | 64 bytes: the 32-byte seed followed by the 32-byte public key | 32 bytes: the raw Ed25519 public key |
///
/// A P-256 secret key is the 32-byte scalar with leading zero bytes removed, so about one
/// key in 256 is shorter than 32 bytes. Do not check for an exact length or store it in a
/// fixed-size slot; SLIM also accepts the scalar left-padded to 32 bytes.
///
/// Neither key is PKCS#8, PEM, DER or base64 encoded, and the public key carries no
/// DER wrapper. SLIM generates the pair and hands it to
/// [`IdentityProvider::set_signature_keys`]; store and return those exact bytes
/// from [`IdentityProvider::get_signature_keys`] and [`IdentityProvider::export_identity`].
/// A provider that supplies its own keys must produce them in the format above.
///
/// How a token carries the public key, for [`IdentityVerifier::get_public_key`] and
/// [`IdentityVerifier::verify_key_binding`], is up to the scheme. If you put it in a claim
/// as text, use standard base64 (RFC 4648 section 4, with `=` padding).
#[derive(uniffi::Record, Clone, Debug)]
pub struct SignatureKeys {
    /// Private signing key, in the format described on [`SignatureKeys`].
    pub secret_key: Vec<u8>,
    /// Public key, in the format described on [`SignatureKeys`].
    pub public_key: Vec<u8>,
}

/// Generate a fresh signature key pair, in the format described on [`SignatureKeys`],
/// for the ciphersuite this library was built with.
///
/// An [`IdentityProvider`] needs a key pair from the start (see
/// [`IdentityProvider::get_signature_keys`]), so a provider with no key of its own can
/// create one here instead of implementing the crypto itself.
///
/// ```python
/// keys = slim_bindings.generate_signature_keys()
/// ```
#[uniffi::export]
pub fn generate_signature_keys() -> Result<SignatureKeys, SlimError> {
    let (secret_key, public_key) = slim_auth::utils::generate_mls_signature_keys()?;
    Ok(SignatureKeys {
        secret_key,
        public_key,
    })
}

/// A provider's persistable identity, saved so the app can be restored verbatim
/// after a restart. `credential` is sensitive: it ends up in the persisted store
/// (encrypted with the persistence passphrase), so treat it as a secret.
#[derive(uniffi::Record, Clone, Debug)]
pub struct IdentityExport {
    /// Stable identity string.
    pub id: String,
    /// Provider credential material (e.g. a shared secret). Sensitive.
    pub credential: Vec<u8>,
    /// MLS signature secret key bytes, exactly as in [`SignatureKeys::secret_key`]
    /// (raw bytes, not PKCS#8 or base64). Sensitive.
    pub signature_secret_key: Vec<u8>,
    /// MLS signature public key bytes, exactly as in [`SignatureKeys::public_key`].
    pub signature_public_key: Vec<u8>,
}

impl From<ExportedIdentity> for IdentityExport {
    fn from(e: ExportedIdentity) -> Self {
        Self {
            id: e.id,
            credential: e.credential,
            signature_secret_key: e.signature_secret_key,
            signature_public_key: e.signature_public_key,
        }
    }
}

impl From<IdentityExport> for ExportedIdentity {
    fn from(e: IdentityExport) -> Self {
        Self {
            id: e.id,
            credential: e.credential,
            signature_secret_key: e.signature_secret_key,
            signature_public_key: e.signature_public_key,
        }
    }
}

/// Proves this app's identity to its peers. Implemented in the foreign language.
///
/// `initialize` is async. The other methods are called synchronously from SLIM worker
/// threads (including from MLS), so they must not block for long and must be safe to
/// call concurrently; prefetch and cache anything that needs I/O.
///
/// # Keys and tokens
///
/// SLIM signs the control messages of every session with the provider's signature
/// key pair, with or without MLS, and the peer checks that signature with the key its
/// [`IdentityVerifier::get_public_key`] returns. A provider therefore must:
///
/// 1. hold a signature key pair from the start (create one with
///    [`generate_signature_keys`] if you have none) and return it from
///    [`IdentityProvider::get_signature_keys`];
/// 2. return tokens that let the paired verifier recover its current public key and check
///    that the token is bound to it;
/// 3. replace the pair when [`IdentityProvider::set_signature_keys`] is called (MLS
///    does this) and carry the new public key in the next tokens.
///
/// If the verifier cannot return the signer's key, new sessions fail with "identity
/// verification failed".
///
/// # One provider object per app
///
/// SLIM clones its handle to the provider for every session of an app, and all clones
/// call the same object, so the app and its sessions share one key pair. That is intended,
/// but it also means two apps given the same object share one key slot: rotating the keys
/// in one changes the other's. Give each app its own provider object. (Built-in identities
/// created from a config differ: a shared-secret identity shares its keys across clones
/// like this, while a JWT signer keeps a key pair per clone.)
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait IdentityProvider: Send + Sync {
    /// One-time asynchronous setup (e.g. loading credentials, contacting an identity
    /// service), awaited while an app is created, before any other method. SLIM calls it
    /// once per app creation, so it runs again if you reuse the object for another app:
    /// make it idempotent. Use one provider object per app anyway (see below).
    async fn initialize(&self) -> Result<(), IdentityCallbackError>;

    /// Return a token proving this app's identity. Called every time a token is
    /// needed, so implementations should cache and refresh as appropriate.
    fn get_token(&self) -> Result<String, IdentityCallbackError>;

    /// Return the identity (subject) the token carries.
    fn get_id(&self) -> Result<String, IdentityCallbackError>;

    /// The signature key pair currently in use. SLIM signs session control messages
    /// with it, so it must always be available.
    ///
    /// Return the bytes you were given, unchanged (see [`SignatureKeys`] for the format).
    ///
    /// ```python
    /// def get_signature_keys(self):
    ///     if self._keys is None:
    ///         raise slim_bindings.IdentityCallbackError.Failed("no keys installed yet")
    ///     return slim_bindings.SignatureKeys(self._keys.secret, self._keys.public)
    /// ```
    fn get_signature_keys(&self) -> Result<SignatureKeys, IdentityCallbackError>;

    /// Whether SLIM's keys were installed through
    /// [`IdentityProvider::set_signature_keys`] or [`IdentityProvider::restore_identity`].
    /// Return false until then, even if you created a pair yourself: when MLS is used,
    /// SLIM generates and installs a pair only while this is false. If this fails, SLIM
    /// does not treat it as "not installed", which would replace the keys every session of
    /// the app shares: MLS setup for that session fails instead.
    fn mls_signature_keys_installed(&self) -> Result<bool, IdentityCallbackError>;

    /// Called when MLS is used, with the MLS signature key pair generated by SLIM.
    /// Both keys are raw bytes in the format described on [`SignatureKeys`]: keep them
    /// byte-for-byte and return them unchanged from `get_signature_keys`.
    /// Subsequent tokens should carry `public_key` (e.g. as a `pubkey` or `cnf` claim) so
    /// peers can recover it with [`IdentityVerifier::get_public_key`] and check the binding
    /// with [`IdentityVerifier::verify_key_binding`]. Replace your current pair with these.
    ///
    /// ```python
    /// def set_signature_keys(self, private_key, public_key):
    ///     self._keys = Keys(private_key, public_key)
    ///     self._token = self._mint_token(claims={"pubkey": base64.b64encode(public_key)})
    /// ```
    fn set_signature_keys(
        &self,
        private_key: Vec<u8>,
        public_key: Vec<u8>,
    ) -> Result<(), IdentityCallbackError>;

    /// The persistable identity, or `None` if this provider does not support
    /// persistence. Used when the app is created with persistence enabled. If this fails,
    /// SLIM logs the error and does not update the session record, rather than saving it next
    /// to a stale identity. Make `export_identity` reliable: MLS group state is still saved.
    ///
    /// ```python
    /// def export_identity(self):
    ///     return slim_bindings.IdentityExport(
    ///         id=self._id,
    ///         credential=self._secret,          # sensitive: stored encrypted
    ///         signature_secret_key=self._keys.secret,
    ///         signature_public_key=self._keys.public,
    ///     )
    ///
    /// def restore_identity(self, identity):
    ///     self._id, self._secret = identity.id, identity.credential
    ///     self._keys = Keys(identity.signature_secret_key, identity.signature_public_key)
    ///     self._installed = True
    /// ```
    fn export_identity(&self) -> Result<Option<IdentityExport>, IdentityCallbackError>;

    /// Whether the MLS signing key is long-term, so persisted group state would
    /// expose it. When true, persistence requires a passphrase. If this fails, SLIM
    /// assumes it does when opening a persistent store, and fails to restore a persisted
    /// MLS group.
    fn holds_long_term_signing_key(&self) -> Result<bool, IdentityCallbackError>;

    /// Adopt an identity returned earlier by [`IdentityProvider::export_identity`],
    /// so the provider presents the same id and MLS keys as before a restart. The
    /// keys count as installed afterwards.
    ///
    /// Be all-or-nothing: on failure leave the provider as it was. SLIM keeps using the
    /// same object when the restore fails, so a half-applied restore would leak into it.
    /// Any failure turns persistence off and leaves the stored identity alone: with a
    /// fresh identity, SLIM would install new MLS keys, and the persisted groups were
    /// created with the old ones. [`IdentityCallbackError::IdentityMismatch`] says the
    /// persisted identity is not yours, which is logged as such.
    fn restore_identity(&self, identity: IdentityExport) -> Result<(), IdentityCallbackError>;
}

/// Verifies the identity of peers. Implemented in the foreign language.
///
/// `initialize` and `verify_async` are async, so they may do I/O. The other methods
/// are called synchronously from SLIM worker threads (including from MLS), so they
/// must not block for long and must be safe to call concurrently; prefetch and cache
/// anything they need.
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait IdentityVerifier: Send + Sync {
    /// One-time asynchronous setup (e.g. fetching trust bundles or keys), awaited
    /// once while the app is being created, before any other method.
    async fn initialize(&self) -> Result<(), IdentityCallbackError>;

    /// Verify the token where an async context is available. Fail if it is invalid
    /// or expired.
    async fn verify_async(&self, token: String) -> Result<(), IdentityCallbackError>;

    /// Verify the token from a synchronous context. Fail if it is invalid or expired.
    fn verify(&self, token: String) -> Result<(), IdentityCallbackError>;

    /// Verify the token and return its claims as a JSON object.
    ///
    /// Include a `sub` claim with the subject the token identifies. SLIM binds the name id of
    /// a peer to its subject, and with `SLIM_IDENTITY_BINDING=enforce` it drops a message
    /// whose id does not derive from it. Without a `sub` (or `id`) claim there is no
    /// subject to bind to, so none of those checks run for the peer.
    fn get_claims(&self, token: String) -> Result<String, IdentityCallbackError>;

    /// Verify the token and return the subject it identifies.
    fn get_subject(&self, token: String) -> Result<String, IdentityCallbackError>;

    /// Verify the token and return the public key its holder signs session control messages
    /// with, as raw bytes in the format described on [`SignatureKeys`]. SLIM checks the
    /// signature of a control message with this key, so it must be the key the sender's
    /// [`IdentityProvider::get_signature_keys`] returns. Where the token carries the key is
    /// up to your scheme.
    ///
    /// ```python
    /// def get_public_key(self, token):
    ///     claims = self._decode_and_verify(token)
    ///     return base64.b64decode(claims["pubkey"])
    /// ```
    fn get_public_key(&self, token: String) -> Result<Vec<u8>, IdentityCallbackError>;

    /// Verify the token and check that it is bound to `public_key`, the MLS
    /// signature public key of the peer, as raw bytes in the format described on
    /// [`SignatureKeys`]. What "bound" means is up to your scheme (a claim holding the key,
    /// a DPoP `cnf`/`jkt` thumbprint, ...). If the token carries the key as a base64
    /// string, compare after decoding it, not the other way round. Fail with
    /// [`IdentityCallbackError::PublicKeyMismatch`] if bound to a different key.
    ///
    /// Only MLS calls this. If your scheme has no MLS support, fail with
    /// [`IdentityCallbackError::Failed`]; MLS then reports the error, and sessions without
    /// MLS are unaffected. [`IdentityVerifier::get_public_key`] is different: sessions need
    /// it, with or without MLS.
    ///
    /// ```python
    /// def verify_key_binding(self, token, public_key):
    ///     claims = self._decode_and_verify(token)
    ///     if base64.b64decode(claims["pubkey"]) != public_key:
    ///         raise slim_bindings.IdentityCallbackError.PublicKeyMismatch("token bound to another key")
    /// ```
    fn verify_key_binding(
        &self,
        token: String,
        public_key: Vec<u8>,
    ) -> Result<(), IdentityCallbackError>;
}

/// Adapts a foreign [`IdentityProvider`] to the core [`TokenProvider`]. Cloning shares the
/// foreign object, which is where the keys and tokens live.
#[derive(Clone)]
struct ForeignProvider {
    inner: Arc<dyn IdentityProvider>,
}

impl TokenProvider for ForeignProvider {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        Ok(self.inner.initialize().await?)
    }

    fn get_token(&self) -> Result<String, AuthError> {
        Ok(self.inner.get_token()?)
    }

    fn get_id(&self) -> Result<String, AuthError> {
        Ok(self.inner.get_id()?)
    }

    fn get_signature_keys(&self) -> Result<(Vec<u8>, Vec<u8>), AuthError> {
        let keys = self.inner.get_signature_keys()?;
        Ok((keys.secret_key, keys.public_key))
    }

    fn mls_signature_keys_installed(&self) -> Result<bool, AuthError> {
        Ok(self.inner.mls_signature_keys_installed()?)
    }

    async fn set_signature_keys(
        &mut self,
        private_key: Vec<u8>,
        public_key: Vec<u8>,
    ) -> Result<(), AuthError> {
        Ok(self.inner.set_signature_keys(private_key, public_key)?)
    }

    fn export_identity(&self) -> Result<Option<ExportedIdentity>, AuthError> {
        Ok(self.inner.export_identity()?.map(Into::into))
    }

    fn holds_long_term_signing_key(&self) -> Result<bool, AuthError> {
        Ok(self.inner.holds_long_term_signing_key()?)
    }

    fn with_restored_identity(self, identity: ExportedIdentity) -> Result<Self, AuthError> {
        self.inner.restore_identity(identity.into())?;
        Ok(self)
    }
}

/// Adapts a foreign [`IdentityVerifier`] to the core [`Verifier`].
#[derive(Clone)]
struct ForeignVerifier {
    inner: Arc<dyn IdentityVerifier>,
}

impl Verifier for ForeignVerifier {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        Ok(self.inner.initialize().await?)
    }

    async fn verify(&self, token: impl AsRef<str> + Send) -> Result<(), AuthError> {
        Ok(self.inner.verify_async(token.as_ref().to_string()).await?)
    }

    fn try_verify(&self, token: impl AsRef<str>) -> Result<(), AuthError> {
        Ok(self.inner.verify(token.as_ref().to_string())?)
    }

    async fn get_claims<Claims>(&self, token: impl AsRef<str> + Send) -> Result<Claims, AuthError>
    where
        Claims: DeserializeOwned + Send,
    {
        self.try_get_claims(token)
    }

    fn try_get_claims<Claims>(&self, token: impl AsRef<str>) -> Result<Claims, AuthError>
    where
        Claims: DeserializeOwned + Send,
    {
        let claims = self.inner.get_claims(token.as_ref().to_string())?;
        Ok(serde_json::from_str(&claims)?)
    }

    async fn get_public_key(&self, token: impl AsRef<str> + Send) -> Result<Vec<u8>, AuthError> {
        Ok(self.inner.get_public_key(token.as_ref().to_string())?)
    }

    fn try_get_subject(&self, token: impl AsRef<str>) -> Result<String, AuthError> {
        Ok(self.inner.get_subject(token.as_ref().to_string())?)
    }

    fn try_verify_key_binding(
        &self,
        token: impl AsRef<str>,
        public_key: &[u8],
    ) -> Result<(), AuthError> {
        let token = token.as_ref();
        self.inner
            .verify_key_binding(token.to_string(), public_key.to_vec())
            .map_err(|err| match err {
                // Fill in the details the foreign side cannot know.
                IdentityCallbackError::PublicKeyMismatch { message } => {
                    AuthError::PublicKeyMismatch {
                        subject: self.try_get_subject(token).unwrap_or_else(|e| {
                            tracing::warn!(error = %e, "cannot read the subject of a token whose key does not match");
                            String::new()
                        }),
                        expected: IdentityClaims::encode_public_key(public_key),
                        found: message,
                    }
                }
                other => other.into(),
            })
    }
}

/// Wrap foreign implementations into the core provider/verifier types.
pub(crate) fn into_core(
    provider: Arc<dyn IdentityProvider>,
    verifier: Arc<dyn IdentityVerifier>,
) -> (AuthProvider, AuthVerifier) {
    (
        AuthProvider::custom(ForeignProvider { inner: provider }),
        AuthVerifier::custom(ForeignVerifier { inner: verifier }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_auth::traits::{TokenProvider, Verifier};

    #[derive(Default)]
    struct Provider {
        keys: parking_lot::Mutex<Option<SignatureKeys>>,
        restored: parking_lot::Mutex<Option<IdentityExport>>,
    }

    #[async_trait::async_trait]
    impl IdentityProvider for Provider {
        async fn initialize(&self) -> Result<(), IdentityCallbackError> {
            Ok(())
        }
        fn get_token(&self) -> Result<String, IdentityCallbackError> {
            Ok("tok".into())
        }
        fn get_id(&self) -> Result<String, IdentityCallbackError> {
            Ok("alice".into())
        }
        fn get_signature_keys(&self) -> Result<SignatureKeys, IdentityCallbackError> {
            self.keys
                .lock()
                .clone()
                .ok_or(IdentityCallbackError::Failed {
                    message: "no keys".into(),
                })
        }
        fn mls_signature_keys_installed(&self) -> Result<bool, IdentityCallbackError> {
            Ok(self.keys.lock().is_some())
        }
        fn set_signature_keys(
            &self,
            secret_key: Vec<u8>,
            public_key: Vec<u8>,
        ) -> Result<(), IdentityCallbackError> {
            *self.keys.lock() = Some(SignatureKeys {
                secret_key,
                public_key,
            });
            Ok(())
        }
        fn export_identity(&self) -> Result<Option<IdentityExport>, IdentityCallbackError> {
            let Some(keys) = self.keys.lock().clone() else {
                return Ok(None);
            };
            Ok(Some(IdentityExport {
                id: "alice".into(),
                credential: b"cred".to_vec(),
                signature_secret_key: keys.secret_key,
                signature_public_key: keys.public_key,
            }))
        }
        fn holds_long_term_signing_key(&self) -> Result<bool, IdentityCallbackError> {
            Ok(true)
        }
        fn restore_identity(&self, identity: IdentityExport) -> Result<(), IdentityCallbackError> {
            if identity.id != "alice" {
                return Err(IdentityCallbackError::IdentityMismatch {
                    message: format!("persisted identity is {}", identity.id),
                });
            }
            *self.keys.lock() = Some(SignatureKeys {
                secret_key: identity.signature_secret_key.clone(),
                public_key: identity.signature_public_key.clone(),
            });
            *self.restored.lock() = Some(identity);
            Ok(())
        }
    }

    struct Verif;

    #[async_trait::async_trait]
    impl IdentityVerifier for Verif {
        async fn initialize(&self) -> Result<(), IdentityCallbackError> {
            Ok(())
        }
        async fn verify_async(&self, token: String) -> Result<(), IdentityCallbackError> {
            self.verify(token)
        }
        fn verify(&self, token: String) -> Result<(), IdentityCallbackError> {
            if token == "tok" {
                Ok(())
            } else {
                Err(IdentityCallbackError::Failed {
                    message: "bad token".into(),
                })
            }
        }
        fn get_claims(&self, token: String) -> Result<String, IdentityCallbackError> {
            self.verify(token)?;
            Ok(r#"{"sub":"alice"}"#.into())
        }
        fn get_subject(&self, token: String) -> Result<String, IdentityCallbackError> {
            self.verify(token)?;
            Ok("alice".into())
        }
        fn get_public_key(&self, token: String) -> Result<Vec<u8>, IdentityCallbackError> {
            self.verify(token)?;
            Ok(b"key".to_vec())
        }
        fn verify_key_binding(
            &self,
            token: String,
            key: Vec<u8>,
        ) -> Result<(), IdentityCallbackError> {
            self.verify(token)?;
            if key == b"key" {
                Ok(())
            } else {
                Err(IdentityCallbackError::PublicKeyMismatch {
                    message: "other".into(),
                })
            }
        }
    }

    /// A restore that rejects the persisted identity as not its own reaches the session
    /// layer as `AuthError::IdentityMismatch`, which is what turns persistence off.
    #[test]
    fn restore_identity_mismatch_keeps_its_type() {
        let (provider, _) = into_core(Arc::new(Provider::default()), Arc::new(Verif));
        let foreign = IdentityExport {
            id: "someone-else".into(),
            credential: vec![],
            signature_secret_key: vec![1],
            signature_public_key: vec![2],
        };
        match provider.with_restored_identity(foreign.into()) {
            Err(AuthError::IdentityMismatch(message)) => assert!(message.contains("someone-else")),
            other => panic!("expected IdentityMismatch, got {other:?}"),
        }
    }

    /// A provider whose callbacks fail gets an error back, never a made-up answer or a
    /// panic across the FFI boundary.
    #[test]
    fn failing_callbacks_are_reported() {
        struct Broken;

        #[async_trait::async_trait]
        impl IdentityProvider for Broken {
            async fn initialize(&self) -> Result<(), IdentityCallbackError> {
                Ok(())
            }
            fn get_token(&self) -> Result<String, IdentityCallbackError> {
                Ok("tok".into())
            }
            fn get_id(&self) -> Result<String, IdentityCallbackError> {
                Ok("id".into())
            }
            fn get_signature_keys(&self) -> Result<SignatureKeys, IdentityCallbackError> {
                Err(failed())
            }
            fn mls_signature_keys_installed(&self) -> Result<bool, IdentityCallbackError> {
                Err(failed())
            }
            fn set_signature_keys(
                &self,
                _: Vec<u8>,
                _: Vec<u8>,
            ) -> Result<(), IdentityCallbackError> {
                Err(failed())
            }
            fn export_identity(&self) -> Result<Option<IdentityExport>, IdentityCallbackError> {
                Err(failed())
            }
            fn holds_long_term_signing_key(&self) -> Result<bool, IdentityCallbackError> {
                Err(failed())
            }
            fn restore_identity(&self, _: IdentityExport) -> Result<(), IdentityCallbackError> {
                Err(failed())
            }
        }

        fn failed() -> IdentityCallbackError {
            IdentityCallbackError::Failed {
                message: "boom".into(),
            }
        }

        let (provider, _) = into_core(Arc::new(Broken), Arc::new(Verif));
        assert!(provider.mls_signature_keys_installed().is_err());
        assert!(provider.export_identity().is_err());
        assert!(provider.holds_long_term_signing_key().is_err());
    }

    #[tokio::test]
    async fn foreign_identity_round_trip() {
        let (mut provider, mut verifier) =
            into_core(Arc::new(Provider::default()), Arc::new(Verif));
        provider.initialize().await.unwrap();
        verifier.initialize().await.unwrap();
        verifier.verify("tok").await.unwrap();
        assert!(verifier.verify("nope").await.is_err());

        assert_eq!(provider.get_id().unwrap(), "alice");
        verifier.try_verify(provider.get_token().unwrap()).unwrap();
        assert!(verifier.try_verify("nope").is_err());
        assert_eq!(verifier.try_get_subject("tok").unwrap(), "alice");
        assert_eq!(verifier.get_public_key("tok").await.unwrap(), b"key");

        assert!(!provider.mls_signature_keys_installed().unwrap());
        provider
            .set_signature_keys(vec![1], b"key".to_vec())
            .await
            .unwrap();
        assert!(provider.mls_signature_keys_installed().unwrap());
        assert_eq!(provider.get_signature_keys().unwrap().1, b"key");

        // Persistence hooks are forwarded.
        assert!(provider.holds_long_term_signing_key().unwrap());
        let exported = provider
            .export_identity()
            .unwrap()
            .expect("exports identity");
        assert_eq!(exported.credential, b"cred");
        let restored = provider.with_restored_identity(exported).unwrap();
        assert_eq!(restored.get_signature_keys().unwrap().1, b"key");

        verifier.try_verify_key_binding("tok", b"key").unwrap();
        match verifier.try_verify_key_binding("tok", b"x") {
            Err(AuthError::PublicKeyMismatch { subject, .. }) => assert_eq!(subject, "alice"),
            other => panic!("expected PublicKeyMismatch, got {other:?}"),
        }
    }
}
