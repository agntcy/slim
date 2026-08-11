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
use slim_datapath::api::ProtoName as Name;
use slim_service::app::App as SlimApp;
use slim_session::context::SessionContext;
use slim_session::errors::SessionError;
use slim_session::{AppChannelReceiver, CompletionHandle};

use super::{RpcCode, RpcError, STATUS_CODE_KEY};

/// Received message from a session
#[derive(Debug, Clone)]
pub struct ReceivedMessage {
    /// Message metadata
    pub metadata: std::collections::HashMap<String, String>,
    /// Message payload
    pub payload: Vec<u8>,
    /// Name of the app that sent this message (extracted from the SLIM header)
    pub source: Name,
}

impl ReceivedMessage {
    /// Returns `true` when this message is an end-of-stream marker:
    /// status code is `Ok` **and** the payload is empty.
    pub fn is_eos(&self) -> bool {
        RpcCode::from_metadata_str(self.metadata.get(STATUS_CODE_KEY).map(String::as_str))
            == RpcCode::Ok
            && self.payload.is_empty()
    }
}

/// Session transmitter - used only for sending messages
#[derive(Clone)]
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
    pub fn source(&self) -> &Name {
        self.controller.source()
    }

    /// Get the destination name
    pub fn destination(&self) -> &Name {
        self.controller.dst()
    }

    /// Get session metadata
    pub fn metadata(&self) -> std::collections::HashMap<String, String> {
        self.controller.metadata()
    }

    /// Publish a message to `target` through this session.
    ///
    /// Pass `self.destination()` for broadcast behaviour, or pass the
    /// requester's source name to unicast a reply directly to the caller.
    pub async fn publish(
        &self,
        target: &Name,
        data: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<std::collections::HashMap<String, String>>,
    ) -> Result<CompletionHandle, RpcError> {
        self.controller
            .publish(target, data, payload_type, metadata)
            .await
            .map_err(|e| RpcError::internal(e.chain().to_string()))
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

        match app.delete_session(self.controller.as_ref()) {
            Ok(handle) => {
                handle.await.map_err(|e| {
                    RpcError::internal(format!("Failed to delete session: {}", e.chain()))
                })?;
                tracing::debug!(session_id = %self.controller.id(), "Successfully deleted session");
            }
            Err(e) => {
                tracing::warn!(session_id = %self.controller.id(), error = %e, "Failed to delete session");
            }
        }

        Ok(())
    }
}

impl SessionRx {
    /// Receive a message from the session with optional timeout.
    ///
    /// Returns `Err(SessionError::SessionClosed)` when the channel is gone,
    /// `Err(SessionError::ReceiveTimeout)` on timeout, and the raw
    /// `SessionError` for all other session-level errors (e.g.
    /// `ParticipantDisconnected`).  Callers can therefore distinguish
    /// transient membership events from fatal failures.
    pub async fn get_message(
        &mut self,
        timeout: Option<Duration>,
    ) -> Result<ReceivedMessage, SessionError> {
        let recv_future = async {
            let msg = self.rx.recv().await.ok_or(SessionError::SessionClosed)??;

            // Extract payload from the proto message
            let payload = if let Some(content) = msg.get_payload() {
                if let Ok(app_payload) = content.as_application_payload() {
                    app_payload.blob.clone()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            let source = msg.get_source();
            Ok(ReceivedMessage {
                metadata: msg.metadata,
                payload,
                source,
            })
        };

        if let Some(timeout_duration) = timeout {
            futures::pin_mut!(recv_future);
            let delay = Delay::new(timeout_duration);
            futures::pin_mut!(delay);

            match futures::future::select(recv_future, delay).await {
                futures::future::Either::Left((result, _)) => result,
                futures::future::Either::Right(_) => Err(SessionError::ReceiveTimeout),
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
