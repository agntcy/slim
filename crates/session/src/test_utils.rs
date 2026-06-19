// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Test utilities and mock implementations for session testing.
//!
//! This module provides mock implementations of traits used in session management
//! for testing purposes. It is only compiled when running tests.

use slim_auth::errors::AuthError;
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::{Participant, ProtoMessage as Message, ProtoName};
use std::sync::Arc;

use crate::SessionError;
use crate::common::{SessionMessage, SessionOutput};
use crate::traits::MessageHandler;

/// Mock token provider for testing.
#[derive(Clone, Default)]
pub struct MockTokenProvider;

impl TokenProvider for MockTokenProvider {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        Ok(())
    }

    fn get_token(&self) -> Result<String, AuthError> {
        Ok(String::new())
    }

    fn get_id(&self) -> Result<String, AuthError> {
        Ok("mock_id".to_string())
    }

    async fn set_signature_keys(
        &mut self,
        _private_key: Vec<u8>,
        _public_key: Vec<u8>,
    ) -> Result<(), AuthError> {
        Ok(())
    }
}

/// Mock verifier for testing.
#[derive(Clone, Default)]
pub struct MockVerifier;

impl Verifier for MockVerifier {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        Ok(())
    }

    async fn verify(&self, _token: impl AsRef<str> + Send) -> Result<(), AuthError> {
        Ok(())
    }

    fn try_verify(&self, _token: impl AsRef<str>) -> Result<(), AuthError> {
        Ok(())
    }

    async fn get_claims<Claims>(&self, _token: impl AsRef<str> + Send) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned,
    {
        Err(AuthError::TokenInvalid)
    }

    fn try_get_claims<Claims>(&self, _token: impl AsRef<str>) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned,
    {
        Err(AuthError::TokenInvalid)
    }
}

/// Attach identity token and E2E header signature to a control message for tests.
pub fn sign_test_control_message<P>(msg: &mut Message, provider: &P) -> Result<(), SessionError>
where
    P: TokenProvider,
{
    let identity = provider.get_token().map_err(|e| SessionError::Auth(e))?;
    msg.get_slim_header_mut().set_identity(identity);

    let private_key = provider
        .get_signature_secret_key()
        .map_err(|e| SessionError::Auth(e))?;
    let public_key = provider
        .get_signature_public_key()
        .map_err(|e| SessionError::Auth(e))?;
    let aad = crate::mls_state::build_aad(msg);
    let signature = slim_auth::utils::sign_header_aad(&aad, &private_key, &public_key)
        .map_err(|e| SessionError::Auth(e))?;
    msg.get_slim_header_mut().e2e_header_sig = Some(signature);
    Ok(())
}

/// Mock inner message handler for testing.
pub struct MockInnerHandler {
    pub messages_received: Arc<tokio::sync::Mutex<Vec<SessionMessage>>>,
    pub endpoints_added: Arc<tokio::sync::Mutex<Vec<ProtoName>>>,
    pub endpoints_removed: Arc<tokio::sync::Mutex<Vec<ProtoName>>>,
}

impl MockInnerHandler {
    pub fn new() -> Self {
        Self {
            messages_received: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            endpoints_added: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            endpoints_removed: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    pub async fn get_messages_count(&self) -> usize {
        self.messages_received.lock().await.len()
    }

    pub async fn get_endpoints_added_count(&self) -> usize {
        self.endpoints_added.lock().await.len()
    }

    pub async fn get_endpoints_removed_count(&self) -> usize {
        self.endpoints_removed.lock().await.len()
    }
}

impl Default for MockInnerHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageHandler for MockInnerHandler {
    async fn init(&mut self) -> Result<(), SessionError> {
        Ok(())
    }

    async fn on_message(&mut self, message: SessionMessage) -> Result<SessionOutput, SessionError> {
        self.messages_received.lock().await.push(message);
        Ok(SessionOutput::new())
    }

    async fn add_endpoint(
        &mut self,
        endpoint: &Participant,
    ) -> Result<SessionOutput, SessionError> {
        self.endpoints_added.lock().await.push(endpoint.get_name()?);
        Ok(SessionOutput::new())
    }

    fn remove_endpoint(&mut self, endpoint: &ProtoName) {
        let endpoints = self.endpoints_removed.clone();
        let endpoint = endpoint.clone();
        tokio::spawn(async move {
            endpoints.lock().await.push(endpoint);
        });
    }

    fn needs_drain(&self) -> bool {
        false
    }

    async fn on_shutdown(&mut self) -> Result<(), SessionError> {
        Ok(())
    }
}

impl<P, V> crate::traits::MlsStateSelector<P, V> for MockInnerHandler
where
    P: slim_auth::traits::TokenProvider + Send + Sync + Clone + 'static,
    V: slim_auth::traits::Verifier + Send + Sync + Clone + 'static,
{
    fn set_mls_state(
        &mut self,
        _mls_state: std::sync::Arc<parking_lot::Mutex<crate::mls_state::MlsState<P, V>>>,
    ) {
    }
}
