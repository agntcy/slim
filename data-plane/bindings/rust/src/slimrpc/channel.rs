// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Client-side RPC channel implementation
//!
//! Provides a Channel type for making RPC calls to remote services.
//! Supports all gRPC streaming patterns over SLIM sessions.
//!
//! All eight public RPC methods are built on top of two core streaming methods:
//! - `responses_from_request`: for single-request patterns (unary / unary-stream)
//! - `responses_from_stream_input`: for streaming-request patterns (stream-unary / stream-stream)
//!
//! Both core methods accept a `multicast: bool` flag:
//! - `false` (P2P): uses a PointToPoint session; stream ends on server EOS
//! - `true`  (GROUP): uses a Multicast session; individual server EOS messages are
//!   skipped; the stream ends when the GROUP session closes
//!
//! The "unary" variants are simply the streaming variants followed by `.next()`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_stream::try_stream;
use display_error_chain::ErrorChainExt;
use futures::StreamExt;
use futures::future::join_all;
use futures::stream::Stream;
use futures_timer::Delay;
use parking_lot::RwLock as ParkingRwLock;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_datapath::api::ProtoSessionType;
use slim_datapath::messages::Name;
use slim_service::app::App as SlimApp;
use slim_session::errors::SessionError;
use slim_session::notification::Notification;

use super::{
    BidiStreamHandler, Context, METHOD_KEY, Metadata, RPC_ID_KEY, ReceivedMessage,
    RequestStreamWriter, ResponseStreamReader, RpcCode, RpcError, SERVICE_KEY, STATUS_CODE_KEY,
    calculate_deadline, calculate_timeout_duration,
    codec::{Decoder, Encoder},
    msg_is_terminal,
    session_wrapper::{SessionRx, SessionTx, new_session},
};

/// Metadata key used to tag client-sent messages on a multicast session.
///
/// GROUP (Multicast) sessions echo every published message back to the sender.
/// The dispatcher skips messages carrying this key so echoes of client requests
/// are never forwarded to the caller's response channel.
const RPC_DIR_KEY: &str = "_rpc_dir";

/// Value that marks a message as a client-sent multicast request.
const RPC_DIR_REQ: &str = "req";

// ── ResponseDispatcher ────────────────────────────────────────────────────────

/// Routes incoming response messages to the correct per-RPC mpsc channel.
///
/// The background `response_dispatcher_task` calls `dispatch()` for each
/// message it receives from the shared `SessionRx`. RPC callers register
/// before sending their request and unregister after receiving all responses.
struct ResponseDispatcher {
    pending: ParkingRwLock<HashMap<String, mpsc::UnboundedSender<ReceivedMessage>>>,
    /// If set, messages not claimed by any active RPC call are forwarded here
    /// instead of being dropped. Used by `Channel::subscribe_group_inbox`.
    unrouted_tx: ParkingRwLock<Option<mpsc::UnboundedSender<ReceivedMessage>>>,
}

impl ResponseDispatcher {
    fn new() -> Self {
        Self {
            pending: ParkingRwLock::new(HashMap::new()),
            unrouted_tx: ParkingRwLock::new(None),
        }
    }

    fn register(&self, rpc_id: &str) -> mpsc::UnboundedReceiver<ReceivedMessage> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.pending.write().insert(rpc_id.to_string(), tx);
        rx
    }

    fn unregister(&self, rpc_id: &str) {
        self.pending.write().remove(rpc_id);
    }

    /// Install a sink for messages that don't match any active RPC call.
    ///
    /// Replaces any previously installed sink. The old sink (and its receiver)
    /// are implicitly closed when the sender is dropped.
    fn set_unrouted_sink(&self, tx: mpsc::UnboundedSender<ReceivedMessage>) {
        *self.unrouted_tx.write() = Some(tx);
    }

    /// Forward a message to the channel registered for `rpc_id`.
    ///
    /// Returns `true` on delivery to an active RPC caller, `false` otherwise.
    /// When `false`, the message is forwarded to the unrouted sink (if one is
    /// installed) so that GROUP session observers can receive it.
    fn dispatch(&self, msg: ReceivedMessage, rpc_id: &str) -> bool {
        let lock = self.pending.read();
        if let Some(tx) = lock.get(rpc_id) {
            tx.send(msg).is_ok()
        } else {
            drop(lock);
            if let Some(tx) = self.unrouted_tx.read().as_ref() {
                let _ = tx.send(msg);
            }
            false
        }
    }

    /// Drop all pending registrations.
    ///
    /// Called when the underlying session closes. Dropping every sender causes
    /// each waiting `rx.recv().await` to return `None`, cleanly ending the
    /// response loop in all in-flight multicast callers.
    fn close_all(&self) {
        self.pending.write().clear();
    }
}

// ── DispatcherGuard ───────────────────────────────────────────────────────────

/// RAII guard that calls `dispatcher.unregister(rpc_id)` on drop.
///
/// Ensures the per-RPC channel is always removed from the dispatcher even
/// when a stream is abandoned early (e.g. after a single `.next()` call).
struct DispatcherGuard {
    dispatcher: Arc<ResponseDispatcher>,
    rpc_id: String,
}

impl Drop for DispatcherGuard {
    fn drop(&mut self) {
        self.dispatcher.unregister(&self.rpc_id);
    }
}

// ── Dispatcher task ───────────────────────────────────────────────────────────

/// Reads every message from `session_rx` and routes it by `rpc-id`.
///
/// Messages carrying `_rpc_dir = "req"` are echoes of the client's own
/// multicast publishes (GROUP sessions echo back to the sender) and are
/// silently dropped. On session close `close_all()` is called so any in-flight
/// multicast receive loop unblocks with `None`.
async fn response_dispatcher_task(mut session_rx: SessionRx, dispatcher: Arc<ResponseDispatcher>) {
    loop {
        match session_rx.get_message(None).await {
            Ok(msg) => {
                // Drop echoes of client-sent multicast requests.
                if msg.metadata.get(RPC_DIR_KEY).map(String::as_str) == Some(RPC_DIR_REQ) {
                    continue;
                }
                let rpc_id = msg.metadata.get(RPC_ID_KEY).cloned().unwrap_or_default();
                if !dispatcher.dispatch(msg, &rpc_id) {
                    tracing::trace!(%rpc_id, "Received message for unknown rpc-id (forwarded to group inbox if subscribed)");
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "Response dispatcher: session closed");
                dispatcher.close_all();
                break;
            }
        }
    }
}

// ── ChannelSession ────────────────────────────────────────────────────────────

/// A live SLIM session together with its dispatcher and background task.
struct ChannelSession {
    tx: SessionTx,
    dispatcher: Arc<ResponseDispatcher>,
    /// Background task reading from the session. When finished the session
    /// is considered dead and will be recreated on the next RPC call.
    task: JoinHandle<()>,
}

impl ChannelSession {
    fn is_alive(&self) -> bool {
        !self.task.is_finished()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn generate_rpc_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Metadata for the first (or only) message of an RPC call.
fn first_msg_metadata(ctx: &Context, service: &str, method: &str, rpc_id: &str) -> Metadata {
    let mut meta = ctx.metadata();
    meta.insert(SERVICE_KEY.to_string(), service.to_string());
    meta.insert(METHOD_KEY.to_string(), method.to_string());
    meta.insert(RPC_ID_KEY.to_string(), rpc_id.to_string());
    meta
}

/// Metadata for a stream continuation message (not the first).
fn continuation_metadata(rpc_id: &str) -> Metadata {
    let mut meta = HashMap::new();
    meta.insert(RPC_ID_KEY.to_string(), rpc_id.to_string());
    meta
}

/// Metadata for an end-of-stream marker.
fn eos_metadata(rpc_id: &str, routing: Option<(&Context, &str, &str)>) -> Metadata {
    let mut meta = match routing {
        Some((ctx, service, method)) => first_msg_metadata(ctx, service, method, rpc_id),
        None => continuation_metadata(rpc_id),
    };
    let code: i32 = RpcCode::Ok.into();
    meta.insert(STATUS_CODE_KEY.to_string(), code.to_string());
    meta
}

/// Like `first_msg_metadata` but tags the message with `_rpc_dir = "req"` so
/// the dispatcher can drop the GROUP session echo.
fn multicast_first_msg_metadata(
    ctx: &Context,
    service: &str,
    method: &str,
    rpc_id: &str,
) -> Metadata {
    let mut meta = first_msg_metadata(ctx, service, method, rpc_id);
    meta.insert(RPC_DIR_KEY.to_string(), RPC_DIR_REQ.to_string());
    meta
}

/// Like `continuation_metadata` but tagged with `_rpc_dir = "req"`.
fn multicast_continuation_metadata(rpc_id: &str) -> Metadata {
    let mut meta = continuation_metadata(rpc_id);
    meta.insert(RPC_DIR_KEY.to_string(), RPC_DIR_REQ.to_string());
    meta
}

/// Like `eos_metadata` but tagged with `_rpc_dir = "req"`.
fn multicast_eos_metadata(rpc_id: &str, routing: Option<(&Context, &str, &str)>) -> Metadata {
    let mut meta = eos_metadata(rpc_id, routing);
    meta.insert(RPC_DIR_KEY.to_string(), RPC_DIR_REQ.to_string());
    meta
}

// ── Channel ───────────────────────────────────────────────────────────────────

/// Notification receiver type for incoming group session invites.
type NotificationsRx =
    Arc<tokio::sync::Mutex<mpsc::Receiver<Result<Notification, SessionError>>>>;

/// Client-side channel for making RPC calls.
///
/// Manages two persistent SLIM sessions to the same remote:
/// - A `PointToPoint` session for standard unary/stream RPC calls.
/// - A `Multicast` (GROUP) session for multicast RPC calls that broadcast to
///   all group members and stream back every individual response.
///
/// Both sessions are lazily initialised and recreated when dead.
///
/// When `notifications` is set the channel operates in **passive-join** mode:
/// instead of creating its own GROUP session it waits for an incoming session
/// invite (a `Notification::NewSession` on the notifications channel) before
/// the first multicast operation. Use `Channel::invite_participant` (on the
/// moderator side) to invite this channel's app into the GROUP session.
#[derive(Clone, uniffi::Object)]
pub struct Channel {
    app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
    remote: Name,
    connection_id: Option<u64>,
    runtime: tokio::runtime::Handle,
    /// Persistent P2P session (lazily initialised, recreated when dead).
    session: Arc<ParkingRwLock<Option<ChannelSession>>>,
    /// Persistent GROUP/Multicast session (lazily initialised on first multicast call).
    multicast_session: Arc<ParkingRwLock<Option<ChannelSession>>>,
    /// If set, the channel is in passive-join mode: the GROUP session is
    /// obtained by waiting for a `NewSession` notification rather than by
    /// creating a new session.  Multiple clones share the same mutex-protected
    /// receiver so only the first waiter receives each notification.
    notifications: Option<NotificationsRx>,
}

impl Channel {
    pub fn new_internal(app: Arc<SlimApp<AuthProvider, AuthVerifier>>, remote: Name) -> Self {
        Self::new_with_connection_internal(app, remote, None, None)
    }

    /// Create a channel that joins an existing GROUP session passively.
    ///
    /// The returned channel does NOT create its own GROUP session. When the
    /// first multicast operation is performed (or `subscribe_group_inbox` is
    /// called) it reads `notifications` until a `NewSession` event arrives,
    /// then uses that session.
    ///
    /// The caller is responsible for arranging the invite: the moderator of
    /// the GROUP session must call `Channel::invite_participant` with the name
    /// of the app associated with this channel.
    pub fn new_internal_with_notifications(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        remote: Name,
        notifications: mpsc::Receiver<Result<Notification, SessionError>>,
    ) -> Self {
        let mut ch = Self::new_internal(app, remote);
        ch.notifications = Some(Arc::new(tokio::sync::Mutex::new(notifications)));
        ch
    }

    pub fn new_with_connection_internal(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        remote: Name,
        connection_id: Option<u64>,
        runtime: Option<tokio::runtime::Handle>,
    ) -> Self {
        let runtime = runtime.unwrap_or_else(|| {
            tokio::runtime::Handle::try_current()
                .expect("No tokio runtime found. Either provide a runtime handle or call from within a tokio runtime context")
        });

        Self {
            app,
            remote,
            connection_id,
            runtime,
            session: Arc::new(ParkingRwLock::new(None)),
            multicast_session: Arc::new(ParkingRwLock::new(None)),
            notifications: None,
        }
    }

    // ── Session management ────────────────────────────────────────────────────

    /// Return the existing session in `slot` if alive, otherwise create a new
    /// one of the given `session_type`.
    ///
    /// Fast path: read lock — return if alive.
    /// Slow path: create outside any lock, then write lock with re-check.
    async fn get_or_create_session_in(
        &self,
        slot: &Arc<ParkingRwLock<Option<ChannelSession>>>,
        session_type: ProtoSessionType,
        mls_enabled: bool,
    ) -> Result<(SessionTx, Arc<ResponseDispatcher>), RpcError> {
        {
            let guard = slot.read();
            if let Some(ref cs) = *guard
                && cs.is_alive()
            {
                return Ok((cs.tx.clone(), cs.dispatcher.clone()));
            }
        }

        tracing::debug!(?session_type, "no persistent session — recreating");
        let (session_tx, session_rx) =
            self.create_raw_session_typed(session_type, mls_enabled).await?;
        let dispatcher = Arc::new(ResponseDispatcher::new());
        let task = tokio::spawn(response_dispatcher_task(session_rx, dispatcher.clone()));
        let new_cs = ChannelSession {
            tx: session_tx.clone(),
            dispatcher: dispatcher.clone(),
            task,
        };

        let mut guard = slot.write();
        if let Some(ref existing) = *guard
            && existing.is_alive()
        {
            new_cs.task.abort();
            return Ok((existing.tx.clone(), existing.dispatcher.clone()));
        }
        *guard = Some(new_cs);
        Ok((session_tx, dispatcher))
    }

    async fn get_or_create_session(
        &self,
    ) -> Result<(SessionTx, Arc<ResponseDispatcher>), RpcError> {
        self.get_or_create_session_in(
            &self.session,
            ProtoSessionType::PointToPoint,
            true,
        )
        .await
    }

    async fn get_or_create_multicast_session(
        &self,
    ) -> Result<(SessionTx, Arc<ResponseDispatcher>), RpcError> {
        self.get_or_create_session_in(
            &self.multicast_session,
            ProtoSessionType::Multicast,
            false,
        )
        .await
    }

    /// Wait for an incoming GROUP session invite and use that session.
    ///
    /// Reads `self.notifications` until a `NewSession` notification for a
    /// Multicast session arrives, then sets up the dispatcher and background
    /// task exactly as `get_or_create_session_in` does.
    ///
    /// Returns an error if no notifications channel is configured or if the
    /// channel closes before an invite arrives.
    async fn join_incoming_group_session(
        &self,
    ) -> Result<(SessionTx, Arc<ResponseDispatcher>), RpcError> {
        // Fast path: already have a live session.
        {
            let guard = self.multicast_session.read();
            if let Some(ref cs) = *guard
                && cs.is_alive()
            {
                return Ok((cs.tx.clone(), cs.dispatcher.clone()));
            }
        }

        let notifications = self
            .notifications
            .as_ref()
            .ok_or_else(|| RpcError::internal("Channel has no notifications receiver for passive group join"))?;

        let session_ctx = tokio::time::timeout(Duration::from_secs(10), async {
            let mut lock = notifications.lock().await;
            loop {
                match lock.recv().await {
                    Some(Ok(Notification::NewSession(ctx))) => return Ok(ctx),
                    Some(_) => continue,
                    None => return Err(RpcError::internal("Notifications channel closed before receiving group session invite")),
                }
            }
        })
        .await
        .map_err(|_| RpcError::deadline_exceeded("Timed out waiting for group session invite"))??;

        let (session_tx, session_rx) = new_session(session_ctx);
        let dispatcher = Arc::new(ResponseDispatcher::new());
        let task = tokio::spawn(response_dispatcher_task(session_rx, dispatcher.clone()));
        let new_cs = ChannelSession {
            tx: session_tx.clone(),
            dispatcher: dispatcher.clone(),
            task,
        };

        let mut guard = self.multicast_session.write();
        if let Some(ref existing) = *guard
            && existing.is_alive()
        {
            new_cs.task.abort();
            return Ok((existing.tx.clone(), existing.dispatcher.clone()));
        }
        *guard = Some(new_cs);
        Ok((session_tx, dispatcher))
    }

    /// Invite a participant into the GROUP session.
    ///
    /// Creates the GROUP session (as moderator) if not yet established, then
    /// sends a discovery-request invite to `destination`. The call returns as
    /// soon as the invite message is dispatched; use a short sleep after this
    /// call to allow all invited participants to finish joining before making
    /// multicast RPC calls.
    ///
    /// # Arguments
    /// * `destination` – Name of the app to invite. If multiple apps are
    ///   subscribed to the same name all of them will receive the invite.
    pub async fn invite_participant(&self, destination: Name) -> Result<(), RpcError> {
        let (session_tx, _) = self.get_or_create_multicast_session().await?;
        session_tx
            .controller()
            .invite_participant(&destination)
            .await
            .map_err(|e| {
                RpcError::internal(format!("Failed to invite {}: {}", destination, e))
            })?;
        Ok(())
    }

    /// Return the P2P or GROUP session depending on `multicast`.
    async fn get_session_for_mode(
        &self,
        multicast: bool,
    ) -> Result<(SessionTx, Arc<ResponseDispatcher>), RpcError> {
        if multicast {
            self.get_or_create_multicast_session().await
        } else {
            self.get_or_create_session().await
        }
    }

    /// Create a raw SLIM session to the remote peer.
    async fn create_raw_session_typed(
        &self,
        session_type: ProtoSessionType,
        mls_enabled: bool,
    ) -> Result<(SessionTx, SessionRx), RpcError> {
        let app = self.app.clone();
        let remote = self.remote.clone();
        let connection_id = self.connection_id;
        let runtime = &self.runtime;

        let handle = runtime.spawn(async move {
            if let Some(conn_id) = connection_id {
                tracing::debug!(
                    remote = %remote,
                    connection_id = conn_id,
                    "Setting route before creating session"
                );
                if let Err(e) = app.set_route(&remote, conn_id).await {
                    tracing::warn!(
                        remote = %remote,
                        connection_id = conn_id,
                        error = %e,
                        "Failed to set route"
                    );
                }
            }

            tracing::debug!(remote = %remote, ?session_type, "Creating persistent session");

            let (max_retries, interval) = if session_type == ProtoSessionType::Multicast {
                (3, Duration::from_millis(200))
            } else {
                (10, Duration::from_secs(1))
            };

            let slim_config = slim_session::session_config::SessionConfig {
                session_type,
                mls_enabled,
                max_retries: Some(max_retries),
                interval: Some(interval),
                initiator: true,
                metadata: HashMap::new(),
            };

            let (session_ctx, completion) = app
                .create_session(slim_config, remote.clone(), None)
                .await
                .map_err(|e| RpcError::unavailable(format!("Failed to create session: {}", e)))?;

            completion
                .await
                .map_err(|e| RpcError::unavailable(format!("Session handshake failed: {}", e)))?;

            Ok(new_session(session_ctx))
        });

        handle
            .await
            .map_err(|e| RpcError::internal(format!("Session creation task failed: {}", e)))?
    }

    // ── Stream helpers ────────────────────────────────────────────────────────

    fn parse_status_code(&self, code_str: Option<&String>) -> Result<RpcCode, RpcError> {
        match code_str {
            Some(s) => s
                .parse::<i32>()
                .ok()
                .and_then(|code| RpcCode::try_from(code).ok())
                .ok_or(RpcError::internal(format!("Invalid status code: {}", s))),
            None => Ok(RpcCode::Ok),
        }
    }

    // ── Send helpers ──────────────────────────────────────────────────────────

    /// Send a single request message. Adds `_rpc_dir = "req"` when `multicast`.
    async fn send_request_impl<Req>(
        &self,
        session: &SessionTx,
        ctx: &Context,
        request: Req,
        service_name: &str,
        method_name: &str,
        rpc_id: &str,
        multicast: bool,
    ) -> Result<(), RpcError>
    where
        Req: Encoder,
    {
        let request_bytes = request.encode()?;
        let meta = if multicast {
            multicast_first_msg_metadata(ctx, service_name, method_name, rpc_id)
        } else {
            first_msg_metadata(ctx, service_name, method_name, rpc_id)
        };
        let handle = session
            .publish(request_bytes, Some("msg".to_string()), Some(meta))
            .await?;
        tracing::debug!(%service_name, %method_name, %rpc_id, %multicast, "Sent request");
        handle.await.map_err(|e| {
            RpcError::internal(format!(
                "Failed to complete sending {}-{}: {}",
                service_name, method_name, e.chain()
            ))
        })?;
        Ok(())
    }

    /// Send a stream of request messages. Adds `_rpc_dir = "req"` on every
    /// message when `multicast`.
    async fn send_request_stream_impl<Req>(
        &self,
        session: &SessionTx,
        ctx: &Context,
        request_stream: impl Stream<Item = Req> + Send + 'static,
        service_name: &str,
        method_name: &str,
        rpc_id: &str,
        multicast: bool,
    ) -> Result<(), RpcError>
    where
        Req: Encoder,
    {
        let mut request_stream = std::pin::pin!(request_stream);
        let mut handles = vec![];
        let mut first = true;
        while let Some(request) = request_stream.next().await {
            let request_bytes = request.encode()?;
            let msg_meta = if first {
                first = false;
                if multicast {
                    multicast_first_msg_metadata(ctx, service_name, method_name, rpc_id)
                } else {
                    first_msg_metadata(ctx, service_name, method_name, rpc_id)
                }
            } else if multicast {
                multicast_continuation_metadata(rpc_id)
            } else {
                continuation_metadata(rpc_id)
            };
            let handle = session
                .publish(request_bytes, Some("msg".to_string()), Some(msg_meta))
                .await?;
            handles.push(handle);
        }
        let routing = if first {
            Some((ctx, service_name, method_name))
        } else {
            None
        };
        let eos_meta = if multicast {
            multicast_eos_metadata(rpc_id, routing)
        } else {
            eos_metadata(rpc_id, routing)
        };
        let handle = session
            .publish(Vec::new(), Some("msg".to_string()), Some(eos_meta))
            .await?;
        handles.push(handle);

        let results = join_all(handles).await;
        for result in results {
            result.map_err(|e| {
                RpcError::internal(format!(
                    "Failed to complete sending {}-{}: {}",
                    service_name,
                    method_name,
                    ErrorChainExt::chain(&e)
                ))
            })?;
        }
        Ok(())
    }

    // ── Core streaming methods ────────────────────────────────────────────────

    /// Core streaming method for single-request patterns.
    ///
    /// Sends `request` once and streams back every response:
    /// - `multicast = false` (P2P): stream ends when the server sends an EOS marker.
    /// - `multicast = true` (GROUP): individual server EOS markers are skipped;
    ///   the stream ends when the GROUP session closes.
    ///
    /// A `DispatcherGuard` ensures the per-RPC channel is always unregistered,
    /// even when the stream is abandoned early (e.g. after a single `.next()`).
    fn responses_from_request<Req, Res>(
        &self,
        multicast: bool,
        service_name: &str,
        method_name: &str,
        request: Req,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> impl Stream<Item = Result<Res, RpcError>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        let service_name = service_name.to_string();
        let method_name = method_name.to_string();
        let channel = self.clone();

        try_stream! {
            let ctx = channel.create_context_for_rpc(timeout, metadata.clone());
            let timeout_duration = calculate_timeout_duration(timeout);
            let mut delay = Delay::new(timeout_duration);

            let (session_tx, dispatcher) = tokio::select! {
                result = channel.get_session_for_mode(multicast) => result,
                _ = &mut delay => Err(RpcError::deadline_exceeded("Client deadline exceeded during session setup")),
            }?;

            let rpc_id = generate_rpc_id();
            let mut rx = dispatcher.register(&rpc_id);
            // Ensures dispatcher.unregister is called even on early drop / error.
            let _guard = DispatcherGuard { dispatcher: dispatcher.clone(), rpc_id: rpc_id.clone() };

            let send_result = tokio::select! {
                result = channel.send_request_impl(
                    &session_tx, &ctx, request,
                    &service_name, &method_name, &rpc_id, multicast,
                ) => result,
                _ = &mut delay => Err(RpcError::deadline_exceeded("Client deadline exceeded while sending request")),
            };
            send_result?;

            loop {
                let received_opt = tokio::select! {
                    msg = rx.recv() => Ok(msg),
                    _ = &mut delay => Err(RpcError::deadline_exceeded(
                        "Client deadline exceeded while receiving response"
                    )),
                };

                let received = match received_opt {
                    Err(e) => Err(e)?,
                    // GROUP session closed → the multicast stream is finished.
                    Ok(None) if multicast => break,
                    // P2P session closed unexpectedly.
                    Ok(None) => Err(RpcError::internal("Response channel closed"))?,
                    Ok(Some(m)) => m,
                };

                if msg_is_terminal(&received) {
                    if multicast {
                        // Skip individual server EOS — other members may still respond.
                        continue;
                    } else {
                        // P2P: server EOS signals end of this response stream.
                        break;
                    }
                }

                let code = channel.parse_status_code(received.metadata.get(STATUS_CODE_KEY))?;
                if code != RpcCode::Ok {
                    let message = String::from_utf8_lossy(&received.payload).to_string();
                    Err(RpcError::new(code, message))?;
                }

                let response = Res::decode(received.payload)?;
                yield response;
            }
            // _guard drops here → dispatcher.unregister(&rpc_id)
        }
    }

    /// Core streaming method for streaming-request patterns.
    ///
    /// Sends `request_stream` (with a trailing EOS marker) concurrently while
    /// receiving responses:
    /// - `multicast = false` (P2P): stream ends when the server sends an EOS marker.
    /// - `multicast = true` (GROUP): individual server EOS markers are skipped;
    ///   the stream ends when the GROUP session closes.
    fn responses_from_stream_input<Req, Res>(
        &self,
        multicast: bool,
        service_name: &str,
        method_name: &str,
        request_stream: impl Stream<Item = Req> + Send + 'static,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> impl Stream<Item = Result<Res, RpcError>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        let service_name = service_name.to_string();
        let method_name = method_name.to_string();
        let channel = self.clone();

        try_stream! {
            let ctx = channel.create_context_for_rpc(timeout, metadata.clone());
            let timeout_duration = calculate_timeout_duration(timeout);
            let mut delay = Delay::new(timeout_duration);

            let (session_tx, dispatcher) = tokio::select! {
                result = channel.get_session_for_mode(multicast) => result,
                _ = &mut delay => Err(RpcError::deadline_exceeded("Client deadline exceeded during session setup")),
            }?;

            let rpc_id = generate_rpc_id();
            let mut rx = dispatcher.register(&rpc_id);
            let _guard = DispatcherGuard { dispatcher: dispatcher.clone(), rpc_id: rpc_id.clone() };

            // Send the request stream in a background task so we can receive
            // responses concurrently.
            let session_tx_for_send = session_tx.clone();
            let ctx_for_send = ctx.clone();
            let service_for_send = service_name.clone();
            let method_for_send = method_name.clone();
            let rpc_id_for_send = rpc_id.clone();
            let channel_for_send = channel.clone();
            let mut send_handle = channel.runtime.spawn(async move {
                channel_for_send
                    .send_request_stream_impl(
                        &session_tx_for_send,
                        &ctx_for_send,
                        request_stream,
                        &service_for_send,
                        &method_for_send,
                        &rpc_id_for_send,
                        multicast,
                    )
                    .await
            });

            let mut send_completed = false;
            loop {
                let received_opt = tokio::select! {
                    msg = rx.recv() => Ok(msg),
                    send_result = &mut send_handle, if !send_completed => {
                        send_completed = true;
                        match send_result {
                            Ok(Ok(_)) => continue,
                            Ok(Err(e)) => Err(e),
                            Err(e) => Err(RpcError::internal(format!("Send task panicked: {}", e))),
                        }
                    },
                    _ = &mut delay => Err(RpcError::deadline_exceeded(
                        "Client deadline exceeded while receiving response"
                    )),
                };

                let received = match received_opt {
                    Err(e) => Err(e)?,
                    Ok(None) if multicast => break,
                    Ok(None) => Err(RpcError::internal("Response channel closed"))?,
                    Ok(Some(m)) => m,
                };

                if msg_is_terminal(&received) {
                    if multicast {
                        continue;
                    } else {
                        break;
                    }
                }

                let code = channel.parse_status_code(received.metadata.get(STATUS_CODE_KEY))?;
                if code != RpcCode::Ok {
                    let message = String::from_utf8_lossy(&received.payload).to_string();
                    Err(RpcError::new(code, message))?;
                }

                let response = Res::decode(received.payload)?;
                yield response;
            }
        }
    }

    // ── RPC methods ───────────────────────────────────────────────────────────

    /// Unary RPC: single request → single response.
    ///
    /// Equivalent to `unary_stream(...).next()`.
    pub async fn unary<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request: Req,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Res, RpcError>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        let stream =
            self.responses_from_request(false, service_name, method_name, request, timeout, metadata);
        futures::pin_mut!(stream);
        stream
            .next()
            .await
            .unwrap_or_else(|| Err(RpcError::internal("No response received")))
    }

    /// Unary-stream RPC: single request → stream of responses.
    pub fn unary_stream<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request: Req,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> impl Stream<Item = Result<Res, RpcError>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        self.responses_from_request(false, service_name, method_name, request, timeout, metadata)
    }

    /// Stream-unary RPC: stream of requests → single response.
    ///
    /// Equivalent to `stream_stream(...).next()`.
    pub async fn stream_unary<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request_stream: impl Stream<Item = Req> + Send + 'static,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Res, RpcError>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        let stream = self.responses_from_stream_input(
            false, service_name, method_name, request_stream, timeout, metadata,
        );
        futures::pin_mut!(stream);
        stream
            .next()
            .await
            .unwrap_or_else(|| Err(RpcError::internal("No response received")))
    }

    /// Stream-stream RPC: stream of requests → stream of responses.
    pub fn stream_stream<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request_stream: impl Stream<Item = Req> + Send + 'static,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> impl Stream<Item = Result<Res, RpcError>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        self.responses_from_stream_input(
            false, service_name, method_name, request_stream, timeout, metadata,
        )
    }

    /// Multicast unary: broadcast one request, receive one response per member.
    ///
    /// Each group member is expected to return a single response. Responses are
    /// yielded as they arrive. The stream ends when the GROUP session closes.
    pub fn multicast_unary<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request: Req,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> impl Stream<Item = Result<Res, RpcError>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        self.responses_from_request(true, service_name, method_name, request, timeout, metadata)
    }

    /// Multicast unary-stream: broadcast one request, receive a stream per member.
    ///
    /// Each group member may return multiple responses. All responses from all
    /// members are interleaved in arrival order. The stream ends when the GROUP
    /// session closes.
    ///
    /// Transport is identical to `multicast_unary`; the semantic difference
    /// (one vs many responses per member) lives in the server handler.
    pub fn multicast_unary_stream<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request: Req,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> impl Stream<Item = Result<Res, RpcError>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        self.responses_from_request(true, service_name, method_name, request, timeout, metadata)
    }

    /// Multicast stream-unary: broadcast a request stream, receive one response per member.
    ///
    /// The request stream is broadcast to all members. Each member replies with
    /// a single response after receiving the full request stream. All responses
    /// are yielded as they arrive. The stream ends when the GROUP session closes.
    pub fn multicast_stream_unary<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request_stream: impl Stream<Item = Req> + Send + 'static,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> impl Stream<Item = Result<Res, RpcError>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        self.responses_from_stream_input(
            true, service_name, method_name, request_stream, timeout, metadata,
        )
    }

    /// Multicast stream-stream: broadcast a request stream, receive a stream per member.
    ///
    /// The request stream is broadcast to all members. Each member replies with
    /// its own response stream. All responses from all members are interleaved in
    /// arrival order. The stream ends when the GROUP session closes.
    ///
    /// Transport is identical to `multicast_stream_unary`; the semantic
    /// difference lives in the server handler.
    pub fn multicast_stream_stream<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request_stream: impl Stream<Item = Req> + Send + 'static,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> impl Stream<Item = Result<Res, RpcError>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        self.responses_from_stream_input(
            true, service_name, method_name, request_stream, timeout, metadata,
        )
    }

    // ── Group inbox ───────────────────────────────────────────────────────────

    /// Subscribe to GROUP session messages not claimed by an active outbound RPC call.
    ///
    /// In a GROUP session every message is broadcast to all members. When another
    /// member makes an RPC call, the responses circulate through the GROUP and
    /// arrive at every participant. Responses whose `rpc-id` is not registered
    /// by this member (i.e. responses to another member's call) are normally
    /// silently dropped. Calling this method installs a sink so those messages
    /// are delivered to the returned stream instead.
    ///
    /// **Note**: only one subscriber is supported per GROUP session. Calling
    /// this method again replaces the previous subscription (the old stream will
    /// see no more messages).
    ///
    /// The stream ends when the GROUP session closes.
    pub async fn subscribe_group_inbox(
        &self,
    ) -> Result<impl Stream<Item = ReceivedMessage>, RpcError> {
        let (_, dispatcher) = if self.notifications.is_some() {
            self.join_incoming_group_session().await?
        } else {
            self.get_or_create_multicast_session().await?
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        dispatcher.set_unrouted_sink(tx);
        Ok(async_stream::stream! {
            while let Some(msg) = rx.recv().await {
                yield msg;
            }
        })
    }

    // ── Misc ──────────────────────────────────────────────────────────────────

    fn create_context_for_rpc(
        &self,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> Context {
        let deadline = calculate_deadline(timeout);
        let mut ctx = Context::new();
        ctx.set_deadline(deadline);
        if let Some(meta) = metadata {
            ctx.metadata_mut().extend(meta);
        }
        ctx
    }

    pub fn app(&self) -> &Arc<SlimApp<AuthProvider, AuthVerifier>> {
        &self.app
    }

    pub fn remote(&self) -> &Name {
        &self.remote
    }

    pub fn connection_id(&self) -> Option<u64> {
        self.connection_id
    }
}

// ── UniFFI exports ────────────────────────────────────────────────────────────

#[uniffi::export]
impl Channel {
    #[uniffi::constructor]
    pub fn new(app: std::sync::Arc<crate::App>, remote: std::sync::Arc<crate::Name>) -> Self {
        Self::new_with_connection(app, remote, None)
    }

    #[uniffi::constructor]
    pub fn new_with_connection(
        app: std::sync::Arc<crate::App>,
        remote: std::sync::Arc<crate::Name>,
        connection_id: Option<u64>,
    ) -> Self {
        let slim_app = app.inner_app().clone();
        let slim_name = remote.as_slim_name().clone();
        let runtime = crate::get_runtime().handle().clone();
        Self::new_with_connection_internal(slim_app, slim_name, connection_id, Some(runtime))
    }

    pub fn call_unary(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Vec<u8>, RpcError> {
        crate::get_runtime().block_on(self.call_unary_async(
            service_name,
            method_name,
            request,
            timeout,
            metadata,
        ))
    }

    pub async fn call_unary_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<Vec<u8>, RpcError> {
        self.unary(&service_name, &method_name, request, timeout, metadata)
            .await
    }

    pub fn call_unary_stream(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<std::sync::Arc<ResponseStreamReader>, RpcError> {
        crate::get_runtime().block_on(self.call_unary_stream_async(
            service_name,
            method_name,
            request,
            timeout,
            metadata,
        ))
    }

    pub async fn call_unary_stream_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<std::sync::Arc<ResponseStreamReader>, RpcError> {
        let channel = self.clone();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        crate::get_runtime().spawn(async move {
            let stream = channel.unary_stream::<Vec<u8>, Vec<u8>>(
                &service_name,
                &method_name,
                request,
                timeout,
                metadata,
            );
            futures::pin_mut!(stream);
            while let Some(item) = futures::StreamExt::next(&mut stream).await {
                if tx.send(item).is_err() {
                    break;
                }
            }
        });
        Ok(std::sync::Arc::new(ResponseStreamReader::new(rx)))
    }

    pub fn call_stream_unary(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> std::sync::Arc<RequestStreamWriter> {
        std::sync::Arc::new(RequestStreamWriter::new(
            self.clone(),
            service_name,
            method_name,
            timeout,
            metadata,
        ))
    }

    pub fn call_stream_stream(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
        metadata: Option<HashMap<String, String>>,
    ) -> std::sync::Arc<BidiStreamHandler> {
        std::sync::Arc::new(BidiStreamHandler::new(
            self.clone(),
            service_name,
            method_name,
            timeout,
            metadata,
        ))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_status_code() {
        assert_eq!(RpcCode::try_from(0), Ok(RpcCode::Ok));
        assert_eq!(RpcCode::try_from(13), Ok(RpcCode::Internal));
        assert!(RpcCode::try_from(999).is_err());
    }

    #[test]
    fn test_generate_rpc_id() {
        let id1 = generate_rpc_id();
        let id2 = generate_rpc_id();
        assert_eq!(id1.len(), 36);
        assert_eq!(id2.len(), 36);
        assert_ne!(id1, id2);
    }

    // ── Metadata helpers ──────────────────────────────────────────────────────

    #[test]
    fn test_multicast_first_msg_metadata_has_rpc_dir() {
        let ctx = Context::new();
        let meta = multicast_first_msg_metadata(&ctx, "MySvc", "MyMethod", "rpc-1");
        assert_eq!(meta.get(RPC_DIR_KEY), Some(&RPC_DIR_REQ.to_string()));
        assert_eq!(meta.get(SERVICE_KEY), Some(&"MySvc".to_string()));
        assert_eq!(meta.get(METHOD_KEY), Some(&"MyMethod".to_string()));
        assert_eq!(meta.get(RPC_ID_KEY), Some(&"rpc-1".to_string()));
    }

    #[test]
    fn test_multicast_continuation_metadata_has_rpc_dir() {
        let meta = multicast_continuation_metadata("rpc-2");
        assert_eq!(meta.get(RPC_DIR_KEY), Some(&RPC_DIR_REQ.to_string()));
        assert_eq!(meta.get(RPC_ID_KEY), Some(&"rpc-2".to_string()));
        // Only rpc-id and _rpc_dir — no service/method.
        assert!(!meta.contains_key(SERVICE_KEY));
        assert!(!meta.contains_key(METHOD_KEY));
    }

    #[test]
    fn test_multicast_eos_metadata_has_rpc_dir() {
        let meta = multicast_eos_metadata("rpc-3", None);
        assert_eq!(meta.get(RPC_DIR_KEY), Some(&RPC_DIR_REQ.to_string()));
        assert_eq!(meta.get(RPC_ID_KEY), Some(&"rpc-3".to_string()));
        // Must carry status OK.
        let code: i32 = meta
            .get(STATUS_CODE_KEY)
            .unwrap()
            .parse()
            .expect("status code is an integer");
        assert_eq!(RpcCode::try_from(code), Ok(RpcCode::Ok));
    }

    #[test]
    fn test_multicast_eos_metadata_with_routing() {
        let ctx = Context::new();
        let meta = multicast_eos_metadata("rpc-4", Some((&ctx, "Svc", "Meth")));
        assert_eq!(meta.get(RPC_DIR_KEY), Some(&RPC_DIR_REQ.to_string()));
        assert_eq!(meta.get(SERVICE_KEY), Some(&"Svc".to_string()));
        assert_eq!(meta.get(METHOD_KEY), Some(&"Meth".to_string()));
    }

    #[test]
    fn test_p2p_metadata_has_no_rpc_dir() {
        let ctx = Context::new();
        let meta = first_msg_metadata(&ctx, "Svc", "Meth", "rpc-5");
        assert!(!meta.contains_key(RPC_DIR_KEY));
        let cont = continuation_metadata("rpc-5");
        assert!(!cont.contains_key(RPC_DIR_KEY));
        let eos = eos_metadata("rpc-5", None);
        assert!(!eos.contains_key(RPC_DIR_KEY));
    }

    // ── Echo filtering ────────────────────────────────────────────────────────

    #[test]
    fn test_echo_detection() {
        // A message with _rpc_dir = "req" is our own echo and should be skipped.
        let mut meta = HashMap::new();
        meta.insert(RPC_DIR_KEY.to_string(), RPC_DIR_REQ.to_string());
        let is_echo = meta.get(RPC_DIR_KEY).map(String::as_str) == Some(RPC_DIR_REQ);
        assert!(is_echo, "tagged message should be identified as echo");

        // A server response has no _rpc_dir key.
        let mut resp_meta = HashMap::new();
        resp_meta.insert(RPC_ID_KEY.to_string(), "rpc-6".to_string());
        let is_resp_echo = resp_meta.get(RPC_DIR_KEY).map(String::as_str) == Some(RPC_DIR_REQ);
        assert!(!is_resp_echo, "response should not be identified as echo");
    }

    // ── ResponseDispatcher ────────────────────────────────────────────────────

    #[test]
    fn test_response_dispatcher_close_all() {
        let dispatcher = ResponseDispatcher::new();
        let mut rx1 = dispatcher.register("rpc-a");
        let mut rx2 = dispatcher.register("rpc-b");

        dispatcher.close_all();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            assert!(rx1.recv().await.is_none(), "rx1 should see None after close_all");
            assert!(rx2.recv().await.is_none(), "rx2 should see None after close_all");
        });
    }

    #[test]
    fn test_response_dispatcher_dispatch_and_unregister() {
        let dispatcher = ResponseDispatcher::new();
        let mut rx = dispatcher.register("rpc-x");

        let msg = ReceivedMessage {
            metadata: HashMap::new(),
            payload: vec![1, 2, 3],
        };
        assert!(dispatcher.dispatch(msg, "rpc-x"));
        assert!(!dispatcher.dispatch(
            ReceivedMessage { metadata: HashMap::new(), payload: vec![] },
            "rpc-unknown"
        ));

        dispatcher.unregister("rpc-x");
        assert!(!dispatcher.dispatch(
            ReceivedMessage { metadata: HashMap::new(), payload: vec![] },
            "rpc-x"
        ));

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let received = rx.recv().await;
            assert!(received.is_some());
            assert_eq!(received.unwrap().payload, vec![1, 2, 3]);
        });
    }

    // ── DispatcherGuard ───────────────────────────────────────────────────────

    #[test]
    fn test_dispatcher_guard_unregisters_on_drop() {
        let dispatcher = Arc::new(ResponseDispatcher::new());
        let _rx = dispatcher.register("rpc-guard");
        {
            let _guard = DispatcherGuard {
                dispatcher: dispatcher.clone(),
                rpc_id: "rpc-guard".to_string(),
            };
            // _guard dropped here → unregister called
        }
        // After guard drops, dispatch must fail.
        assert!(!dispatcher.dispatch(
            ReceivedMessage { metadata: HashMap::new(), payload: vec![] },
            "rpc-guard"
        ));
    }

    // ── Group inbox ───────────────────────────────────────────────────────────

    #[test]
    fn test_unrouted_sink_receives_unmatched_messages() {
        let dispatcher = ResponseDispatcher::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        dispatcher.set_unrouted_sink(tx);

        // A message with an unknown rpc-id should reach the unrouted sink.
        let msg = ReceivedMessage {
            metadata: HashMap::new(),
            payload: vec![42],
        };
        assert!(!dispatcher.dispatch(msg, "unknown-rpc"));

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let received = rx.recv().await;
            assert!(received.is_some());
            assert_eq!(received.unwrap().payload, vec![42]);
        });
    }

    #[test]
    fn test_unrouted_sink_not_called_for_known_rpc_id() {
        let dispatcher = ResponseDispatcher::new();
        let (unrouted_tx, mut unrouted_rx) = mpsc::unbounded_channel();
        dispatcher.set_unrouted_sink(unrouted_tx);

        let mut rpc_rx = dispatcher.register("rpc-y");
        let msg = ReceivedMessage { metadata: HashMap::new(), payload: vec![7] };
        assert!(dispatcher.dispatch(msg, "rpc-y")); // delivered to rpc-y, NOT unrouted

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // rpc-y channel has the message.
            let received = rpc_rx.recv().await;
            assert!(received.is_some());
            assert_eq!(received.unwrap().payload, vec![7]);
            // Unrouted sink is empty.
            assert!(unrouted_rx.try_recv().is_err());
        });
    }
}
