// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use slim_auth::traits::StandardClaims;
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::proto::pubsub::v1::Message;
use slim_datapath::messages::utils::SLIM_IDENTITY;

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
    // add an interceptor to the session
    fn add_interceptor(&self, interceptor: Arc<dyn SessionInterceptor + Send + Sync + 'static>);

    // get the interceptors for the session
    fn get_interceptors(&self) -> Vec<Arc<dyn SessionInterceptor + Send + Sync + 'static>>;

    // run all interceptors on a message received from the app
    async fn on_msg_from_app_interceptors(&self, msg: &mut Message) -> Result<(), SessionError> {
        let interceptors = self.get_interceptors();
        for interceptor in interceptors {
            interceptor.on_msg_from_app(msg).await?;
        }
        Ok(())
    }

    // run all interceptors on a message received from slim
    async fn on_msg_from_slim_interceptors(&self, msg: &mut Message) -> Result<(), SessionError> {
        let interceptors = self.get_interceptors();
        for interceptor in interceptors {
            interceptor.on_msg_from_slim(msg).await?;
        }

        Ok(())
    }
}

pub(crate) struct IdentityInterceptor<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    provider: P,
    verifier: V,
}

impl<P, V> IdentityInterceptor<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    pub fn new(provider: P, verifier: V) -> Self {
        Self { provider, verifier }
    }
}

#[async_trait::async_trait]
impl<P, V> SessionInterceptor for IdentityInterceptor<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    async fn on_msg_from_app(&self, msg: &mut Message) -> Result<(), SessionError> {
        // Let's try first to get the identity without an async call
        let identity = match self.provider.try_get_token() {
            Ok(id) => id,
            Err(_) => {
                // If that fails, we can use the async method
                self.provider
                    .get_token()
                    .await
                    .map_err(|e| SessionError::IdentityPushError(e.to_string()))?
            }
        };

        // Add the identity to the message metadata
        msg.insert_metadata(SLIM_IDENTITY.to_string(), identity);

        Ok(())
    }

    async fn on_msg_from_slim(&self, msg: &mut Message) -> Result<(), SessionError> {
        // Extract the identity from the message metadata
        if let Some(identity) = msg.metadata.get(SLIM_IDENTITY) {
            // Verify the identity using the verifier
            match self.verifier.try_verify::<StandardClaims>(identity) {
                Ok(_) => {
                    // Identity is valid, we can proceed
                    Ok(())
                }
                Err(_e) => {
                    // Try async verification if the sync one fails
                    let _claims = self
                        .verifier
                        .verify::<StandardClaims>(identity)
                        .await
                        .map_err(|e| SessionError::IdentityError(e.to_string()))?;

                    // TODO(msardara): do something with the claims if needed

                    Ok(())
                }
            }
        } else {
            return Err(SessionError::IdentityError(
                "identity not found in message metadata".to_string(),
            ));
        }
    }
}
