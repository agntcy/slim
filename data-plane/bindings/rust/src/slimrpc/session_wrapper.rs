// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Session wrapper for SlimRPC operations
//!
//! Provides a lightweight wrapper around SessionContext that exposes the
//! publish and receive operations needed for RPC communication.

use std::sync::Arc;
use std::time::Duration;

use display_error_chain::ErrorChainExt;

use futures_timer::Delay;
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_datapath::api::{ProtoMessage, ProtoSessionMessageType};
use slim_datapath::messages::{Name, utils::SlimHeaderFlags};
use slim_service::app::App as SlimApp;
use slim_session::context::SessionContext;
use slim_session::errors::SessionError;
use slim_session::{AppChannelReceiver, CompletionHandle};

use super::RpcError;

/// Received message from a session
#[derive(Debug, Clone)]
pub struct ReceivedMessage {
    /// Message metadata
    pub metadata: std::collections::HashMap<String, String>,
    /// Message payload
    pub payload: Vec<u8>,
}

/// Session transmitter - used only for sending messages
pub struct SessionTx {
    /// The underlying session controller
    controller: Arc<slim_session::session_controller::SessionController>,
}

/// Session receiver - used only for receiving messages
pub struct SessionRx {
    /// Receiver for incoming messages
    rx: AppChannelReceiver,
}

impl SessionTx {
    /// Get the session ID
    pub fn session_id(&self) -> u32 {
        self.controller.id()
    }

    /// Get the source name
    pub fn source(&self) -> Name {
        self.controller.source().clone()
    }

    /// Get the destination name
    pub fn destination(&self) -> Name {
        self.controller.dst().clone()
    }

    /// Get session metadata
    pub fn metadata(&self) -> std::collections::HashMap<String, String> {
        self.controller.metadata()
    }

    /// Publish a message through this session
    pub async fn publish(
        &self,
        data: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<std::collections::HashMap<String, String>>,
    ) -> Result<CompletionHandle, RpcError> {
        // Use the Message builder to create a proper protocol message
        let ct = payload_type.unwrap_or_else(|| "msg".to_string());

        let flags = SlimHeaderFlags::new(0, None, None, None, None);

        let mut msg = ProtoMessage::builder()
            .source(self.controller.source().clone())
            .destination(self.controller.dst().clone())
            .identity("")
            .flags(flags)
            .session_type(self.controller.session_type())
            .session_message_type(ProtoSessionMessageType::Msg)
            .session_id(self.controller.id())
            .message_id(rand::random::<u32>())
            .application_payload(&ct, data)
            .build_publish()
            .map_err(|e| RpcError::internal(e.chain().to_string()))?;

        if let Some(map) = metadata
            && !map.is_empty()
        {
            msg.set_metadata_map(map);
        }

        let handle = self
            .controller
            .publish_message(msg)
            .await
            .map_err(|e| RpcError::internal(e.chain().to_string()))?;

        Ok(handle)
    }

    /// Get a clone of the underlying session controller
    pub fn controller(&self) -> Arc<slim_session::session_controller::SessionController> {
        self.controller.clone()
    }

    /// Close the session and delete it from the app
    ///
    /// This properly cleans up the session resources by calling app.delete_session().
    /// After calling this, the session should not be used anymore.
    ///
    /// # Arguments
    /// * `app` - The SLIM app instance to delete the session from
    pub async fn close(&self, app: &SlimApp<AuthProvider, AuthVerifier>) -> Result<(), RpcError> {
        tracing::debug!(session_id = %self.controller.id(), "Closing session");

        if let Ok(handle) = app.delete_session(self.controller.as_ref()) {
            handle.await.map_err(|e| {
                RpcError::internal(format!("Failed to delete session: {}", e.chain()))
            })?;
            tracing::debug!(session_id = %self.controller.id(), "Successfully deleted session");
        } else {
            tracing::warn!(session_id = %self.controller.id(), "Failed to delete session");
        }

        Ok(())
    }
}

impl SessionRx {
    /// Receive a message from the session with optional timeout
    pub async fn get_message(
        &mut self,
        timeout: Option<Duration>,
    ) -> Result<ReceivedMessage, RpcError> {
        let recv_future = async {
            let msg = self
                .rx
                .recv()
                .await
                .ok_or_else(|| RpcError::internal("Session closed"))?
                .map_err(|e: SessionError| {
                    RpcError::internal(format!("Receive error: {}", e.chain()))
                })?;

            // Extract payload from the proto message
            let payload = if let Some(content) = msg.get_payload() {
                // Use the helper method to extract application payload
                if let Ok(app_payload) = content.as_application_payload() {
                    app_payload.blob.clone()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            // Extract metadata and payload from the proto message
            Ok(ReceivedMessage {
                metadata: msg.metadata,
                payload,
            })
        };

        if let Some(timeout_duration) = timeout {
            // Use futures-timer for timeout
            futures::pin_mut!(recv_future);
            let delay = Delay::new(timeout_duration);
            futures::pin_mut!(delay);

            match futures::future::select(recv_future, delay).await {
                futures::future::Either::Left((result, _)) => result,
                futures::future::Either::Right(_) => {
                    Err(RpcError::deadline_exceeded("Receive timeout"))
                }
            }
        } else {
            recv_future.await
        }
    }
}

/// Create session transmitter and receiver from a SessionContext
///
/// Returns a tuple of (SessionTx, SessionRx) where:
/// - SessionTx is used only for sending messages
/// - SessionRx is used only for receiving messages
pub fn new_session(ctx: SessionContext) -> (SessionTx, SessionRx) {
    let (session_weak, rx) = ctx.into_parts();
    let controller = session_weak
        .upgrade()
        .expect("Session controller should be available");

    let tx = SessionTx {
        controller: controller.clone(),
    };

    let rx_wrapper = SessionRx { rx };

    (tx, rx_wrapper)
}
