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
use tokio::sync::RwLock;

use super::Status;

/// Received message from a session
#[derive(Debug, Clone)]
pub struct ReceivedMessage {
    /// Message metadata
    pub metadata: std::collections::HashMap<String, String>,
    /// Message payload
    pub payload: Vec<u8>,
}

/// Internal session state shared across clones
struct SessionInner {
    /// The underlying session controller
    controller: Arc<slim_session::session_controller::SessionController>,
    /// Receiver for incoming messages (wrapped in RwLock for concurrent access)
    rx: RwLock<AppChannelReceiver>,
}

/// Thin wrapper around SessionContext for RPC operations
pub struct Session {
    inner: SessionInner,
}

impl Session {
    /// Create a new session wrapper from a SessionContext
    pub fn new(ctx: SessionContext) -> Self {
        let (session_weak, rx) = ctx.into_parts();
        let controller = session_weak
            .upgrade()
            .expect("Session controller should be available");

        Self {
            inner: SessionInner {
                controller,
                rx: RwLock::new(rx),
            },
        }
    }

    /// Get the session ID
    pub async fn session_id(&self) -> u32 {
        self.inner.controller.id()
    }

    /// Get the source name
    pub async fn source(&self) -> Name {
        self.inner.controller.source().clone()
    }

    /// Get the destination name
    pub async fn destination(&self) -> Name {
        self.inner.controller.dst().clone()
    }

    /// Get session metadata
    pub async fn metadata(&self) -> std::collections::HashMap<String, String> {
        self.inner.controller.metadata()
    }

    /// Publish a message through this session
    pub async fn publish(
        &self,
        data: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<std::collections::HashMap<String, String>>,
    ) -> Result<CompletionHandle, Status> {
        // Use the Message builder to create a proper protocol message
        let ct = payload_type.unwrap_or_else(|| "msg".to_string());

        let flags = SlimHeaderFlags::new(0, None, None, None, None);

        let mut msg = ProtoMessage::builder()
            .source(self.inner.controller.source().clone())
            .destination(self.inner.controller.dst().clone())
            .identity("")
            .flags(flags)
            .session_type(self.inner.controller.session_type())
            .session_message_type(ProtoSessionMessageType::Msg)
            .session_id(self.inner.controller.id())
            .message_id(rand::random::<u32>())
            .application_payload(&ct, data)
            .build_publish()
            .map_err(|e| Status::internal(e.chain().to_string()))?;

        if let Some(map) = metadata
            && !map.is_empty()
        {
            msg.set_metadata_map(map);
        }

        let handle = self
            .inner
            .controller
            .publish_message(msg)
            .await
            .map_err(|e| Status::internal(e.chain().to_string()))?;

        Ok(handle)
    }

    /// Receive a message from the session with optional timeout
    pub async fn get_message(&self, timeout: Option<Duration>) -> Result<ReceivedMessage, Status> {
        let recv_future = async {
            let msg = {
                let mut rx = self.inner.rx.write().await;
                rx.recv()
                    .await
                    .ok_or_else(|| Status::internal("Session closed"))?
                    .map_err(|e: SessionError| {
                        Status::internal(format!("Receive error: {}", e.chain()))
                    })?
            };

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
                    Err(Status::deadline_exceeded("Receive timeout"))
                }
            }
        } else {
            recv_future.await
        }
    }

    /// Get a clone of the underlying session controller
    pub async fn controller(&self) -> Arc<slim_session::session_controller::SessionController> {
        self.inner.controller.clone()
    }

    /// Close the session and delete it from the app
    ///
    /// This properly cleans up the session resources by calling app.delete_session().
    /// After calling this, the session should not be used anymore.
    ///
    /// # Arguments
    /// * `app` - The SLIM app instance to delete the session from
    pub async fn close(&self, app: &SlimApp<AuthProvider, AuthVerifier>) -> Result<(), Status> {
        tracing::debug!(session_id = %self.inner.controller.id(), "Closing session");

        if let Ok(handle) = app.delete_session(self.inner.controller.as_ref()) {
            handle.await.map_err(|e| {
                Status::internal(format!("Failed to delete session: {}", e.chain()))
            })?;
            tracing::debug!(session_id = %self.inner.controller.id(), "Successfully deleted session");
        } else {
            tracing::warn!(session_id = %self.inner.controller.id(), "Failed to delete session");
        }

        Ok(())
    }
}
