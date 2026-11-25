// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Type erasure behind `AuthProvider::custom` and `AuthVerifier::custom`.
//!
//! [`TokenProvider`] and [`Verifier`] are generic and async, so they cannot be trait
//! objects, yet the `Custom` variants of `AuthProvider` and `AuthVerifier` have to hold an
//! arbitrary implementation. The handles here close that gap: each owns an implementation
//! behind an [`ArcSwap`] and exposes it through a private, object-safe trait.
//!
//! - **Reads are lock-free.** `get_token`, `get_signature_keys`, `verify`, `get_claims` and
//!   the rest do one atomic load of the current implementation. They run for every message.
//! - **Writes are serialized.** The traits need `&mut self` to initialize, to rotate the MLS
//!   keys and to restore an identity, so those clone the implementation, change the clone and
//!   store it, one writer at a time (a `tokio` mutex that reads never take). Each change runs
//!   exactly once, on the state the previous writer left. That holds for an implementation
//!   whose clones share state too, such as a foreign object behind an `Arc`: two overlapping
//!   writers through the same handle never apply their changes to it at once or twice. A change that fails stores
//!   nothing. `with_restored_identity` is synchronous and so cannot wait for the mutex: it
//!   fails if a write is in progress. It runs after `initialize` has completed and before
//!   any session is built.
//!
//! Cloning an `AuthProvider::Custom` clones the `Arc`, so every clone shares one
//! implementation and one set of keys, the way the clones of a `SharedSecret` do.

use std::sync::Arc;

use std::future::Future;

use arc_swap::ArcSwap;
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;

use crate::errors::AuthError;
use crate::traits::{ExportedIdentity, TokenProvider, Verifier};

/// Apply an async change to the value in `cell`: change a clone and store it. `writers`
/// makes the change run alone, on the value the previous writer stored.
async fn update<T, F, Fut>(
    cell: &ArcSwap<T>,
    writers: &Mutex<()>,
    change: F,
) -> Result<(), AuthError>
where
    T: Clone + Send + Sync,
    F: FnOnce(T) -> Fut + Send,
    Fut: Future<Output = Result<T, AuthError>> + Send,
{
    let _writer = writers.lock().await;
    // Not held across the await: the guard would tie up a slot of the cell meanwhile.
    let current = T::clone(&cell.load());
    let next = change(current).await?;
    cell.store(Arc::new(next));
    Ok(())
}

/// Object-safe view of a [`TokenProvider`]. Not part of the public API: it only has to be
/// nameable from the `Custom` variant of `AuthProvider`.
#[async_trait::async_trait]
pub trait ErasedProvider: Send + Sync {
    async fn init(&self) -> Result<(), AuthError>;
    fn token(&self) -> Result<String, AuthError>;
    fn id(&self) -> Result<String, AuthError>;
    fn keys(&self) -> Result<(Vec<u8>, Vec<u8>), AuthError>;
    fn keys_installed(&self) -> Result<bool, AuthError>;
    async fn set_keys(&self, private_key: Vec<u8>, public_key: Vec<u8>) -> Result<(), AuthError>;
    fn export(&self) -> Result<Option<ExportedIdentity>, AuthError>;
    fn long_term_key(&self) -> Result<bool, AuthError>;
    fn restore(&self, identity: ExportedIdentity) -> Result<(), AuthError>;
}

/// Object-safe view of a [`Verifier`]. Arguments are `&str` and claims are JSON values, since
/// `impl AsRef<str>` and generic return types are not allowed on a trait object.
#[async_trait::async_trait]
pub trait ErasedVerifier: Send + Sync {
    async fn init(&self) -> Result<(), AuthError>;
    fn verify(&self, token: &str) -> Result<(), AuthError>;
    fn claims(&self, token: &str) -> Result<serde_json::Value, AuthError>;
    async fn public_key(&self, token: &str) -> Result<Vec<u8>, AuthError>;
    fn subject(&self, token: &str) -> Result<String, AuthError>;
    fn key_binding(&self, token: &str, public_key: &[u8]) -> Result<(), AuthError>;
}

/// Owns a [`TokenProvider`]; see the module documentation.
pub(crate) struct ProviderHandle<T> {
    inner: ArcSwap<T>,
    writers: Mutex<()>,
}

impl<T> ProviderHandle<T> {
    pub(crate) fn new(provider: T) -> Self {
        Self {
            inner: ArcSwap::from_pointee(provider),
            writers: Mutex::new(()),
        }
    }
}

#[async_trait::async_trait]
impl<T> ErasedProvider for ProviderHandle<T>
where
    T: TokenProvider + Clone + Send + Sync + 'static,
{
    async fn init(&self) -> Result<(), AuthError> {
        update(&self.inner, &self.writers, |mut provider| async move {
            provider.initialize().await?;
            Ok(provider)
        })
        .await
    }

    fn token(&self) -> Result<String, AuthError> {
        self.inner.load().get_token()
    }

    fn id(&self) -> Result<String, AuthError> {
        self.inner.load().get_id()
    }

    fn keys(&self) -> Result<(Vec<u8>, Vec<u8>), AuthError> {
        self.inner.load().get_signature_keys()
    }

    fn keys_installed(&self) -> Result<bool, AuthError> {
        self.inner.load().mls_signature_keys_installed()
    }

    async fn set_keys(&self, private_key: Vec<u8>, public_key: Vec<u8>) -> Result<(), AuthError> {
        update(&self.inner, &self.writers, |mut provider| async move {
            provider.set_signature_keys(private_key, public_key).await?;
            Ok(provider)
        })
        .await
    }

    fn export(&self) -> Result<Option<ExportedIdentity>, AuthError> {
        self.inner.load().export_identity()
    }

    fn long_term_key(&self) -> Result<bool, AuthError> {
        self.inner.load().holds_long_term_signing_key()
    }

    fn restore(&self, identity: ExportedIdentity) -> Result<(), AuthError> {
        let _writer = self.writers.try_lock().map_err(|_| {
            AuthError::Custom(
                "cannot restore an identity while the provider is being changed".into(),
            )
        })?;
        // Nothing is stored on error.
        let restored = T::clone(&self.inner.load()).with_restored_identity(identity)?;
        self.inner.store(Arc::new(restored));
        Ok(())
    }
}

/// Owns a [`Verifier`]; see the module documentation.
pub(crate) struct VerifierHandle<V> {
    inner: ArcSwap<V>,
    writers: Mutex<()>,
}

impl<V> VerifierHandle<V> {
    pub(crate) fn new(verifier: V) -> Self {
        Self {
            inner: ArcSwap::from_pointee(verifier),
            writers: Mutex::new(()),
        }
    }
}

#[async_trait::async_trait]
impl<V> ErasedVerifier for VerifierHandle<V>
where
    V: Verifier + Clone + Send + Sync + 'static,
{
    async fn init(&self) -> Result<(), AuthError> {
        update(&self.inner, &self.writers, |mut verifier| async move {
            verifier.initialize().await?;
            Ok(verifier)
        })
        .await
    }

    fn verify(&self, token: &str) -> Result<(), AuthError> {
        self.inner.load().verify(token)
    }

    fn claims(&self, token: &str) -> Result<serde_json::Value, AuthError> {
        self.inner.load().get_claims(token)
    }

    async fn public_key(&self, token: &str) -> Result<Vec<u8>, AuthError> {
        let verifier = self.inner.load_full();
        verifier.get_public_key(token).await
    }

    fn subject(&self, token: &str) -> Result<String, AuthError> {
        self.inner.load().get_subject(token)
    }

    fn key_binding(&self, token: &str, public_key: &[u8]) -> Result<(), AuthError> {
        self.inner.load().verify_key_binding(token, public_key)
    }
}

/// Lets the shared `dispatch_verifier!` arms in `auth_provider.rs` treat the erased handle
/// like any other verifier.
impl Verifier for Arc<dyn ErasedVerifier> {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        self.init().await
    }

    fn verify(&self, token: impl AsRef<str>) -> Result<(), AuthError> {
        ErasedVerifier::verify(self.as_ref(), token.as_ref())
    }

    fn get_claims<Claims>(&self, token: impl AsRef<str>) -> Result<Claims, AuthError>
    where
        Claims: DeserializeOwned + Send,
    {
        Ok(serde_json::from_value(ErasedVerifier::claims(
            self.as_ref(),
            token.as_ref(),
        )?)?)
    }

    async fn get_public_key(&self, token: impl AsRef<str> + Send) -> Result<Vec<u8>, AuthError> {
        self.public_key(token.as_ref()).await
    }

    fn get_subject(&self, token: impl AsRef<str>) -> Result<String, AuthError> {
        self.subject(token.as_ref())
    }

    fn verify_key_binding(
        &self,
        token: impl AsRef<str>,
        public_key: &[u8],
    ) -> Result<(), AuthError> {
        self.key_binding(token.as_ref(), public_key)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::auth_provider::{AuthProvider, AuthVerifier};
    use parking_lot::RwLock;
    use serde::Deserialize;

    /// A provider whose state lives behind an `Arc`, so clones agree.
    #[derive(Clone, Default)]
    struct Provider {
        state: Arc<RwLock<State>>,
    }

    #[derive(Default)]
    struct State {
        initialized: bool,
        keys: Option<(Vec<u8>, Vec<u8>)>,
        restored: Option<String>,
    }

    impl TokenProvider for Provider {
        async fn initialize(&mut self) -> Result<(), AuthError> {
            self.state.write().initialized = true;
            Ok(())
        }
        fn get_token(&self) -> Result<String, AuthError> {
            Ok("tok".into())
        }
        fn get_id(&self) -> Result<String, AuthError> {
            Ok("id".into())
        }
        fn get_signature_keys(&self) -> Result<(Vec<u8>, Vec<u8>), AuthError> {
            self.state
                .read()
                .keys
                .clone()
                .ok_or(AuthError::MlsNotSupported)
        }
        fn mls_signature_keys_installed(&self) -> Result<bool, AuthError> {
            Ok(self.state.read().keys.is_some())
        }
        async fn set_signature_keys(
            &mut self,
            private_key: Vec<u8>,
            public_key: Vec<u8>,
        ) -> Result<(), AuthError> {
            self.state.write().keys = Some((private_key, public_key));
            Ok(())
        }
        fn with_restored_identity(self, identity: ExportedIdentity) -> Result<Self, AuthError> {
            if identity.id != "id" {
                return Err(AuthError::IdentityMismatch(identity.id));
            }
            self.state.write().restored = Some(identity.id);
            Ok(self)
        }
    }

    #[derive(Clone, Default)]
    struct Verif {
        initialized: Arc<RwLock<bool>>,
    }

    #[derive(Deserialize)]
    struct Claims {
        sub: String,
    }

    impl Verifier for Verif {
        async fn initialize(&mut self) -> Result<(), AuthError> {
            *self.initialized.write() = true;
            Ok(())
        }
        fn verify(&self, token: impl AsRef<str>) -> Result<(), AuthError> {
            if token.as_ref() == "tok" {
                Ok(())
            } else {
                Err(AuthError::TokenInvalid)
            }
        }
        fn get_claims<C>(&self, token: impl AsRef<str>) -> Result<C, AuthError>
        where
            C: serde::de::DeserializeOwned + Send,
        {
            self.verify(token)?;
            Ok(serde_json::from_value(serde_json::json!({ "sub": "id" }))?)
        }
        async fn get_public_key(
            &self,
            token: impl AsRef<str> + Send,
        ) -> Result<Vec<u8>, AuthError> {
            self.verify(token)?;
            Ok(b"key".to_vec())
        }
        fn get_subject(&self, token: impl AsRef<str>) -> Result<String, AuthError> {
            self.verify(token)?;
            Ok("id".into())
        }
        fn verify_key_binding(
            &self,
            token: impl AsRef<str>,
            public_key: &[u8],
        ) -> Result<(), AuthError> {
            self.verify(token)?;
            if public_key == b"key" {
                Ok(())
            } else {
                Err(AuthError::PublicKeyMismatch {
                    subject: "id".into(),
                    expected: String::new(),
                    found: String::new(),
                })
            }
        }
    }

    #[tokio::test]
    async fn provider_handle_dispatches_and_shares_state_across_clones() {
        let inner = Provider::default();
        let state = inner.state.clone();
        let mut provider = AuthProvider::custom(inner);
        let clone = provider.clone();

        provider.initialize().await.unwrap();
        assert!(state.read().initialized);
        assert_eq!(clone.get_token().unwrap(), "tok");
        assert_eq!(clone.get_id().unwrap(), "id");

        assert!(!clone.mls_signature_keys_installed().unwrap());
        provider.set_signature_keys(vec![1], vec![2]).await.unwrap();
        // The clone sees the rotation: all clones share one provider.
        assert!(clone.mls_signature_keys_installed().unwrap());
        assert_eq!(clone.get_signature_keys().unwrap(), (vec![1], vec![2]));
    }

    /// Keeps its state in plain fields, so a clone that is changed and swapped in replaces
    /// the previous state. Writers that overlap would drop each other's changes (entries
    /// from `applied`, or `initialized`) if the handle swapped in a stale clone.
    #[derive(Clone, Default)]
    struct PlainProvider {
        initialized: bool,
        applied: Vec<u8>,
    }

    impl TokenProvider for PlainProvider {
        async fn initialize(&mut self) -> Result<(), AuthError> {
            // Yield so that other writers get in between the clone and the swap.
            tokio::task::yield_now().await;
            self.initialized = true;
            Ok(())
        }
        fn get_token(&self) -> Result<String, AuthError> {
            Ok("tok".into())
        }
        fn get_id(&self) -> Result<String, AuthError> {
            Ok(if self.initialized { "init" } else { "uninit" }.into())
        }
        fn get_signature_keys(&self) -> Result<(Vec<u8>, Vec<u8>), AuthError> {
            Ok((self.applied.clone(), self.applied.clone()))
        }
        async fn set_signature_keys(
            &mut self,
            private_key: Vec<u8>,
            _public_key: Vec<u8>,
        ) -> Result<(), AuthError> {
            tokio::task::yield_now().await;
            self.applied.extend(private_key);
            Ok(())
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_rotations_are_not_lost() {
        let provider = AuthProvider::custom(PlainProvider::default());
        let mut handles = Vec::new();
        for i in 0..32u8 {
            let mut p = provider.clone();
            handles.push(tokio::spawn(async move {
                p.set_signature_keys(vec![i], vec![i]).await.unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        let (mut applied, _) = provider.get_signature_keys().unwrap();
        applied.sort_unstable();
        assert_eq!(applied, (0..32u8).collect::<Vec<_>>());
    }

    /// Clones share the keys, as a foreign object behind an `Arc` does.
    #[derive(Clone, Default)]
    struct SharedProvider {
        keys: Arc<arc_swap::ArcSwap<Vec<u8>>>,
        calls: Arc<std::sync::atomic::AtomicUsize>,
        in_flight: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl TokenProvider for SharedProvider {
        async fn initialize(&mut self) -> Result<(), AuthError> {
            Ok(())
        }
        fn get_token(&self) -> Result<String, AuthError> {
            Ok("tok".into())
        }
        fn get_id(&self) -> Result<String, AuthError> {
            Ok("id".into())
        }
        fn get_signature_keys(&self) -> Result<(Vec<u8>, Vec<u8>), AuthError> {
            let keys = Vec::clone(&self.keys.load());
            Ok((keys.clone(), keys))
        }
        async fn set_signature_keys(
            &mut self,
            private_key: Vec<u8>,
            _public_key: Vec<u8>,
        ) -> Result<(), AuthError> {
            use std::sync::atomic::Ordering::SeqCst;
            self.calls.fetch_add(1, SeqCst);
            // No other change may be running on the shared state meanwhile.
            assert_eq!(
                self.in_flight.fetch_add(1, SeqCst),
                0,
                "overlapping changes"
            );
            // Long enough for the other writer, spawned alongside, to try to get in.
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            self.keys.store(Arc::new(private_key));
            self.in_flight.fetch_sub(1, SeqCst);
            Ok(())
        }
    }

    /// Overlapping writers run one at a time and each change runs once (no re-apply of an
    /// old change over a newer one).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn overlapping_writers_on_shared_state_run_once_each() {
        for _ in 0..3 {
            let inner = SharedProvider::default();
            let calls = inner.calls.clone();
            let provider = AuthProvider::custom(inner);
            let mut first = provider.clone();
            let mut second = provider.clone();
            let a = tokio::spawn(async move {
                first.set_signature_keys(vec![1], vec![1]).await.unwrap();
            });
            let b = tokio::spawn(async move {
                second.set_signature_keys(vec![2], vec![2]).await.unwrap();
            });
            a.await.unwrap();
            b.await.unwrap();
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        }
    }

    /// A slow `initialize` must not undo a rotation that lands while it runs, and the
    /// rotation must not undo `initialize`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn initialize_and_rotation_do_not_undo_each_other() {
        for _ in 0..50 {
            let provider = AuthProvider::custom(PlainProvider::default());
            let mut init = provider.clone();
            let mut rotate = provider.clone();
            let (a, b) = tokio::join!(
                tokio::spawn(async move { init.initialize().await }),
                tokio::spawn(async move { rotate.set_signature_keys(vec![7], vec![7]).await }),
            );
            a.unwrap().unwrap();
            b.unwrap().unwrap();
            assert_eq!(provider.get_id().unwrap(), "init");
            assert_eq!(provider.get_signature_keys().unwrap().0, vec![7]);
        }
    }

    /// Persistence and MLS call the fallible forms; through `AuthProvider::custom` they
    /// must reach the implementation, not fall back to the defaults.
    #[test]
    fn fallible_queries_reach_the_implementation() {
        #[derive(Clone)]
        struct Failing;
        impl TokenProvider for Failing {
            async fn initialize(&mut self) -> Result<(), AuthError> {
                Ok(())
            }
            fn get_token(&self) -> Result<String, AuthError> {
                Ok("tok".into())
            }
            fn get_id(&self) -> Result<String, AuthError> {
                Ok("id".into())
            }
            async fn set_signature_keys(
                &mut self,
                _: Vec<u8>,
                _: Vec<u8>,
            ) -> Result<(), AuthError> {
                Ok(())
            }
            fn export_identity(&self) -> Result<Option<ExportedIdentity>, AuthError> {
                Err(AuthError::Custom("export".into()))
            }
            fn mls_signature_keys_installed(&self) -> Result<bool, AuthError> {
                Err(AuthError::Custom("installed".into()))
            }
        }
        let provider = AuthProvider::custom(Failing);
        assert!(provider.export_identity().is_err());
        assert!(provider.mls_signature_keys_installed().is_err());
    }

    #[test]
    fn restore_applies_or_reports_a_mismatch() {
        let inner = Provider::default();
        let state = inner.state.clone();
        let provider = AuthProvider::custom(inner);
        let identity = |id: &str| ExportedIdentity {
            id: id.into(),
            credential: vec![],
            signature_secret_key: vec![1],
            signature_public_key: vec![2],
        };

        assert!(matches!(
            provider.clone().with_restored_identity(identity("other")),
            Err(AuthError::IdentityMismatch(_))
        ));
        assert!(state.read().restored.is_none());

        let restored = provider.with_restored_identity(identity("id")).unwrap();
        assert_eq!(state.read().restored.as_deref(), Some("id"));
        assert_eq!(restored.get_id().unwrap(), "id");
    }

    #[tokio::test]
    async fn verifier_handle_dispatches() {
        let inner = Verif::default();
        let initialized = inner.initialized.clone();
        let mut verifier = AuthVerifier::custom(inner);

        verifier.initialize().await.unwrap();
        assert!(*initialized.read());
        verifier.verify("tok").unwrap();
        assert!(verifier.verify("nope").is_err());

        let claims: Claims = verifier.get_claims("tok").unwrap();
        assert_eq!(claims.sub, "id");
        assert_eq!(verifier.get_subject("tok").unwrap(), "id");
        assert_eq!(verifier.get_public_key("tok").await.unwrap(), b"key");

        verifier.verify_key_binding("tok", b"key").unwrap();
        assert!(matches!(
            verifier.verify_key_binding("tok", b"x"),
            Err(AuthError::PublicKeyMismatch { .. })
        ));
    }
}
