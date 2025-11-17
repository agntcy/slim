// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::sync::Arc;

// Third-party crates
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::ProtoMessage as Message;

// Local crate
use crate::errors::SessionError;

#[async_trait::async_trait]
pub trait SessionInterceptor {
    // interceptor to be executed when a message is received from the app
    async fn on_msg_from_app(&self, msg: &mut Message) -> Result<(), SessionError>;
    // interceptor to be executed when a message is received from slim
    async fn on_msg_from_slim(&self, msg: &mut Message) -> Result<(), SessionError>;
}

#[async_trait::async_trait]
pub trait SessionInterceptorProvider {
    /// add an interceptor to the session
    fn add_interceptor(&self, interceptor: Arc<dyn SessionInterceptor + Send + Sync + 'static>);

    /// get all interceptors for the session
    fn get_interceptors(&self) -> Vec<Arc<dyn SessionInterceptor + Send + Sync + 'static>>;

    /// run all interceptors on a message received from the app
    async fn on_msg_from_app_interceptors(&self, msg: &mut Message) -> Result<(), SessionError> {
        let interceptors = self.get_interceptors();
        for interceptor in interceptors {
            interceptor.on_msg_from_app(msg).await?;
        }
        Ok(())
    }

    /// run all interceptors on a message received from slim
    async fn on_msg_from_slim_interceptors(&self, msg: &mut Message) -> Result<(), SessionError> {
        let interceptors = self.get_interceptors();
        for interceptor in interceptors {
            interceptor.on_msg_from_slim(msg).await?;
        }

        Ok(())
    }
}

/// IdentityInterceptor is a session interceptor that adds the identity to the message metadata
/// when a message is received from the app, and verifies the identity when a message is received
/// from slim.
pub struct IdentityInterceptor<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    provider: P,
    verifier: V,
}

/// Implementation of the IdentityInterceptor
impl<P, V> IdentityInterceptor<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    pub fn new(provider: P, verifier: V) -> Self {
        Self { provider, verifier }
    }
}

/// Implementation of the SessionInterceptor trait for IdentityInterceptor
/// This interceptor will add the identity to the message metadata when a message is received
/// from the app, and verify the identity when a message is received from slim.
/// If the identity is not found in the message metadata, it will return an error.
/// If the identity verification fails, it will return an error as well.
#[async_trait::async_trait]
impl<P, V> SessionInterceptor for IdentityInterceptor<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    async fn on_msg_from_app(&self, msg: &mut Message) -> Result<(), SessionError> {
        // Let's try first to get the identity without an async call
        let identity = self
            .provider
            .get_token()
            .map_err(|e| SessionError::IdentityPushError(e.to_string()))?;

        // Add the identity to the message metadata
        msg.get_slim_header_mut().set_identity(identity);

        Ok(())
    }

    async fn on_msg_from_slim(&self, msg: &mut Message) -> Result<(), SessionError> {
        // Extract the identity from the message metadata
        let identity = msg.get_slim_header().get_identity();
        // Verify the identity using the verifier
        match self.verifier.try_verify(&identity) {
            Ok(_) => {
                // Identity is valid, we can proceed
                Ok(())
            }
            Err(_e) => {
                // Try async verification if the sync one fails
                self.verifier
                    .verify(&identity)
                    .await
                    .map_err(|e| SessionError::IdentityError(e.to_string()))?;

                // TODO(msardara): do something with the claims if needed

                Ok(())
            }
        }
    }
}
