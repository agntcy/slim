// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Test utilities and mock implementations for session testing.
//!
//! This module provides mock implementations of traits used in session management
//! for testing purposes. It is only compiled when running tests.

use parking_lot::Mutex;
use slim_auth::errors::AuthError;
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::Status;
use slim_datapath::api::ProtoMessage as Message;
use slim_datapath::messages::Name;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::SessionError;
use crate::common::SessionMessage;
use crate::interceptor::{SessionInterceptor, SessionInterceptorProvider};
use crate::traits::{MessageHandler, Transmitter};

/// Mock token provider for testing.
#[derive(Clone, Default)]
pub struct MockTokenProvider;

#[async_trait::async_trait]
impl TokenProvider for MockTokenProvider {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        Ok(())
    }

    fn get_token(&self) -> Result<String, AuthError> {
        Ok("mock_token".to_string())
    }

    fn get_id(&self) -> Result<String, AuthError> {
        Ok("mock_id".to_string())
    }
}

/// Mock verifier for testing.
#[derive(Clone, Default)]
pub struct MockVerifier;

#[async_trait::async_trait]
impl Verifier for MockVerifier {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        Ok(())
    }

    async fn verify(&self, _token: impl Into<String> + Send) -> Result<(), AuthError> {
        Ok(())
    }

    fn try_verify(&self, _token: impl Into<String>) -> Result<(), AuthError> {
        Ok(())
    }

    async fn get_claims<Claims>(
        &self,
        _token: impl Into<String> + Send,
    ) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned,
    {
        Err(AuthError::TokenInvalid("mock".to_string()))
    }

    fn try_get_claims<Claims>(&self, _token: impl Into<String>) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned,
    {
        Err(AuthError::TokenInvalid("mock".to_string()))
    }
}

/// Mock transmitter for testing.
#[derive(Clone)]
pub struct MockTransmitter {
    pub slim_tx: mpsc::UnboundedSender<Result<Message, Status>>,
    pub interceptors: Arc<Mutex<Vec<Arc<dyn SessionInterceptor + Send + Sync + 'static>>>>,
}

impl SessionInterceptorProvider for MockTransmitter {
    fn add_interceptor(&self, interceptor: Arc<dyn SessionInterceptor + Send + Sync + 'static>) {
        self.interceptors.lock().push(interceptor);
    }

    fn get_interceptors(&self) -> Vec<Arc<dyn SessionInterceptor + Send + Sync + 'static>> {
        self.interceptors.lock().clone()
    }
}

#[async_trait::async_trait]
impl Transmitter for MockTransmitter {
    async fn send_to_slim(&self, message: Result<Message, Status>) -> Result<(), SessionError> {
        self.slim_tx
            .send(message)
            .map_err(|_| SessionError::Processing("channel closed".to_string()))
    }

    async fn send_to_app(
        &self,
        _message: Result<Message, SessionError>,
    ) -> Result<(), SessionError> {
        Ok(())
    }
}

/// Mock inner message handler for testing.
pub struct MockInnerHandler {
    pub messages_received: Arc<tokio::sync::Mutex<Vec<SessionMessage>>>,
    pub endpoints_added: Arc<tokio::sync::Mutex<Vec<Name>>>,
    pub endpoints_removed: Arc<tokio::sync::Mutex<Vec<Name>>>,
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

#[async_trait::async_trait]
impl MessageHandler for MockInnerHandler {
    async fn init(&mut self) -> Result<(), SessionError> {
        Ok(())
    }

    async fn on_message(&mut self, message: SessionMessage) -> Result<(), SessionError> {
        self.messages_received.lock().await.push(message);
        Ok(())
    }

    async fn add_endpoint(&mut self, endpoint: &Name) -> Result<(), SessionError> {
        self.endpoints_added.lock().await.push(endpoint.clone());
        Ok(())
    }

    fn remove_endpoint(&mut self, endpoint: &Name) {
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
