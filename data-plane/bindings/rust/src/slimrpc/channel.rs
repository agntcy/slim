// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use super::common::{service_and_method_to_name, DEADLINE_KEY, MAX_TIMEOUT};
use super::context::MessageContext;
use super::error::{Result, SRPCError};
use crate::App as BindingsApp;
use futures::stream::StreamExt;
use slim_datapath::api::ProtoSessionType;
use slim_datapath::messages::Name;
use slim_session::session_controller::SessionController;
use slim_session::SessionConfig;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_stream::Stream;
use tracing::info;

pub struct Channel {
    remote: Name,
    app: Arc<BindingsApp>,
    conn_id: u64,
}

impl Channel {
    pub fn new(remote: Name, app: Arc<BindingsApp>, conn_id: u64) -> Self {
        Self {
            remote,
            app,
            conn_id,
        }
    }

    async fn common_setup(
        &self,
        method: &str,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(
        Name,
        Arc<SessionController>,
        slim_session::context::SessionContext,
        HashMap<String, String>,
    )> {
        let service_name = service_and_method_to_name(&self.remote, method)?;

        info!(
            "Setting route for service {} with conn_id {}",
            service_name, self.conn_id
        );

        // Set route using the stored connection ID
        self.app
            .set_route_async(
                Arc::new(crate::Name::from(&service_name)),
                self.conn_id,
            )
            .await
            .map_err(|e| SRPCError::ParseIdentity(format!("SetRoute failed: {}", e)))?;

        info!("Creating session for service {}", service_name);

        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(10),
            interval: Some(Duration::from_secs(1)),
            mls_enabled: false,
            initiator: true,
            metadata: HashMap::new(),
        };

        // Use internal app to create session
        let internal_app = self.app.inner_app();
        let (session_ctx, init_ack) = internal_app
            .create_session(config, service_name.clone(), None)
            .await
            .map_err(SRPCError::SessionCreationError)?;

        let session = session_ctx
            .session_arc()
            .ok_or_else(|| SRPCError::Session("Failed to get session".to_string()))?;

        init_ack.await.map_err(SRPCError::SessionInit)?;

        Ok((
            service_name,
            session,
            session_ctx,
            metadata.unwrap_or_default(),
        ))
    }

    async fn send_unary(
        &self,
        request: Vec<u8>,
        session: &Arc<SessionController>,
        service_name: &Name,
        mut metadata: HashMap<String, String>,
        deadline: f64,
    ) -> Result<()> {
        metadata.insert(DEADLINE_KEY.to_string(), deadline.to_string());

        session
            .publish(service_name, request, None, Some(metadata))
            .await
            .map_err(SRPCError::PublishError)?;

        Ok(())
    }

    async fn send_stream(
        &self,
        request_stream: Pin<Box<dyn Stream<Item = Vec<u8>> + Send>>,
        session: &Arc<SessionController>,
        service_name: &Name,
        mut metadata: HashMap<String, String>,
        deadline: f64,
    ) -> Result<()> {
        metadata.insert(DEADLINE_KEY.to_string(), deadline.to_string());

        let mut stream = request_stream;
        while let Some(request) = stream.next().await {
            session
                .publish(service_name, request, None, Some(metadata.clone()))
                .await
                .map_err(SRPCError::PublishError)?;
        }

        // Send end of stream
        let mut end_metadata = metadata.clone();
        end_metadata.insert("code".to_string(), "0".to_string());

        session
            .publish(service_name, vec![], None, Some(end_metadata))
            .await
            .map_err(SRPCError::PublishError)?;

        Ok(())
    }

    async fn receive_unary(
        &self,
        session_ctx: &mut slim_session::context::SessionContext,
        deadline: f64,
    ) -> Result<(MessageContext, Vec<u8>)> {
        let timeout = compute_timeout_from_deadline(deadline);

        let result = tokio::time::timeout(timeout, async {
            let msg = session_ctx
                .rx
                .recv()
                .await
                .ok_or_else(|| SRPCError::Session("Channel closed".to_string()))?
                .map_err(|e| SRPCError::Session(e.to_string()))?;

            let msg_ctx = MessageContext::from_message(&msg);

            // Check for error code
            let metadata = msg.get_metadata_map();
            if let Some(code) = metadata.get("code") {
                if code != "0" {
                    return Err(SRPCError::ResponseError(
                        code.parse().unwrap_or(13),
                        "RPC error".to_string(),
                    ));
                }
            }

            let response = msg
                .get_payload()
                .ok_or_else(|| SRPCError::Session("No payload in response".to_string()))?
                .as_application_payload()
                .map_err(|e| {
                    SRPCError::Session(format!("Failed to get application payload: {}", e))
                })?
                .blob
                .clone();

            Ok((msg_ctx, response))
        })
        .await
        .map_err(|_| SRPCError::Timeout("Request timed out".to_string()))??;

        Ok(result)
    }

    async fn receive_stream(
        &self,
        mut session_ctx: slim_session::context::SessionContext,
        deadline: f64,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Vec<u8>>> + Send>>> {
        let timeout = compute_timeout_from_deadline(deadline);
        let timeout_instant = tokio::time::Instant::now() + timeout;

        let stream = async_stream::try_stream! {
            loop {
                match tokio::time::timeout_at(timeout_instant, session_ctx.rx.recv()).await {
                    Ok(Some(Ok(msg))) => {
                        // Check for end of stream
                        let metadata = msg.get_metadata_map();
                        if metadata.get("code") == Some(&"0".to_string())
                            && msg.get_payload().is_none() {
                                break;
                            }

                        let response = msg
                            .get_payload()
                            .ok_or_else(|| SRPCError::Session("No payload in response".to_string()))?
                            .as_application_payload()
                            .map_err(|e| SRPCError::Session(format!("Failed to get application payload: {}", e)))?
                            .blob.clone();

                        yield response;
                    }
                    Ok(Some(Err(e))) => {
                        Err(SRPCError::Session(e.to_string()))?;
                        break;
                    }
                    Ok(None) => break,
                    Err(_) => {
                        Err(SRPCError::Timeout("Stream timed out".to_string()))?;
                        break;
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }

    pub async fn unary_unary(
        &self,
        method: &str,
        request: Vec<u8>,
        timeout: Option<Duration>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<Vec<u8>> {
        let (service_name, session, mut session_ctx, metadata) =
            self.common_setup(method, metadata).await?;
        let deadline = compute_deadline(timeout);

        self.send_unary(request, &session, &service_name, metadata, deadline)
            .await?;

        let (_ctx, response) = self.receive_unary(&mut session_ctx, deadline).await?;

        // Use internal app to delete session
        self.app.inner_app().delete_session(&session).ok();

        Ok(response)
    }

    pub async fn unary_stream(
        &self,
        method: &str,
        request: Vec<u8>,
        timeout: Option<Duration>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Vec<u8>>> + Send>>> {
        let (service_name, session, session_ctx, metadata) =
            self.common_setup(method, metadata).await?;
        let deadline = compute_deadline(timeout);

        self.send_unary(request, &session, &service_name, metadata, deadline)
            .await?;

        let stream = self.receive_stream(session_ctx, deadline).await?;

        Ok(stream)
    }

    pub async fn stream_unary(
        &self,
        method: &str,
        request_stream: Pin<Box<dyn Stream<Item = Vec<u8>> + Send>>,
        timeout: Option<Duration>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<Vec<u8>> {
        let (service_name, session, mut session_ctx, metadata) =
            self.common_setup(method, metadata).await?;
        let deadline = compute_deadline(timeout);

        self.send_stream(request_stream, &session, &service_name, metadata, deadline)
            .await?;

        let (_ctx, response) = self.receive_unary(&mut session_ctx, deadline).await?;

        // Use internal app to delete session
        self.app.inner_app().delete_session(&session).ok();

        Ok(response)
    }

    pub async fn stream_stream(
        &self,
        method: &str,
        request_stream: Pin<Box<dyn Stream<Item = Vec<u8>> + Send>>,
        timeout: Option<Duration>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Vec<u8>>> + Send>>> {
        let (service_name, session, session_ctx, metadata) =
            self.common_setup(method, metadata).await?;
        let deadline = compute_deadline(timeout);

        self.send_stream(request_stream, &session, &service_name, metadata, deadline)
            .await?;

        let stream = self.receive_stream(session_ctx, deadline).await?;

        Ok(stream)
    }
}

fn compute_deadline(timeout: Option<Duration>) -> f64 {
    let timeout = timeout.unwrap_or(Duration::from_secs(MAX_TIMEOUT));
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        + timeout.as_secs_f64()
}

fn compute_timeout_from_deadline(deadline: f64) -> Duration {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    Duration::from_secs_f64((deadline - now).max(0.0))
}
