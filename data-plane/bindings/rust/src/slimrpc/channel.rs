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
//! Both core methods dispatch based on `self.is_group`:
//! - P2P (`is_group = false`): uses a PointToPoint session; stream ends on server EOS
//! - GROUP (`is_group = true`): uses a Multicast session; stream ends when all members
//!   have sent their final response (EOS); member errors are yielded as stream items
//!
//! The "unary" variants are simply the streaming variants followed by `.next()`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_stream::stream;
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

// ── Multicast response types ──────────────────────────────────────────────────

/// Per-message context attached to every item in a multicast response stream.
///
/// Identifies which group member produced the response, allowing callers to
/// correlate responses with their origin when multiple members are involved.
#[derive(Debug, Clone)]
pub struct MessageContext {
    /// SLIM name of the member app that sent this message.
    pub source: Name,
}

/// A single item in a multicast response stream: the decoded response together
/// with the context identifying its origin.
#[derive(Debug, Clone)]
pub struct MulticastItem<T> {
    /// Context identifying the member that produced this response.
    pub context: MessageContext,
    /// The decoded response message.
    pub message: T,
}

use super::{
    BidiStreamHandler, Context, METHOD_KEY, Metadata, MulticastBidiStreamHandler,
    MulticastResponseReader, RPC_ID_KEY, ReceivedMessage, RequestStreamWriter,
    ResponseStreamReader, RpcCode, RpcError, SERVICE_KEY, STATUS_CODE_KEY, calculate_deadline,
    calculate_timeout_duration,
    codec::{Decoder, Encoder},
    msg_is_terminal,
    session_wrapper::{SessionRx, SessionTx, new_session},
};

// ── ResponseDispatcher ────────────────────────────────────────────────────────

/// Routes incoming response messages to the correct per-RPC mpsc channel.
///
/// The background `response_dispatcher_task` calls `dispatch()` for each
/// message it receives from the shared `SessionRx`. RPC callers register
/// before sending their request and unregister after receiving all responses.
struct ResponseDispatcher {
    pending: ParkingRwLock<HashMap<String, mpsc::UnboundedSender<ReceivedMessage>>>,
}

impl ResponseDispatcher {
    fn new() -> Self {
        Self {
            pending: ParkingRwLock::new(HashMap::new()),
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

    /// Forward a message to the channel registered for `rpc_id`.
    ///
    /// Returns `true` on delivery to an active RPC caller, `false` if
    /// no caller is registered for this rpc-id (message is dropped).
    fn dispatch(&self, msg: ReceivedMessage, rpc_id: &str) -> bool {
        let lock = self.pending.read();
        if let Some(tx) = lock.get(rpc_id) {
            tx.send(msg).is_ok()
        } else {
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
/// On session close `close_all()` is called so any in-flight receive loop
/// unblocks with `None`.
async fn response_dispatcher_task(mut session_rx: SessionRx, dispatcher: Arc<ResponseDispatcher>) {
    loop {
        match session_rx.get_message(None).await {
            Ok(msg) => {
                let rpc_id = msg.metadata.get(RPC_ID_KEY).cloned().unwrap_or_default();
                if !dispatcher.dispatch(msg, &rpc_id) {
                    tracing::trace!(%rpc_id, "Received message for unknown rpc-id, dropping");
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

/// A live SLIM session together with its dispatcher, background task, and member set.
struct ChannelSession {
    tx: SessionTx,
    dispatcher: Arc<ResponseDispatcher>,
    /// Background task reading from the session. When finished the session
    /// is considered dead and will be recreated on the next RPC call.
    task: JoinHandle<()>,
    /// Group members currently in this session.
    /// Populated at session creation and extended by `invite_participant`.
    /// Preserved across session recreations.
    members: HashSet<Name>,
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

async fn send_invite(session_tx: &SessionTx, member: &Name) -> Result<(), RpcError> {
    session_tx
        .controller()
        .invite_participant(member)
        .await
        .map_err(|e| RpcError::internal(format!("Failed to invite {}: {}", member, e)))?
        .await
        .map_err(|e| RpcError::internal(format!("Failed to invite {}: {}", member, e)))
}

/// Generate a random group session name derived from the client name.
///
/// Uses the first two components of `client_name` and a UUID as the third,
/// so the group address lives in the same namespace as the client app.
fn generate_group_name(client_name: &Name) -> Name {
    let parts = client_name.components_strings();
    Name::from_strings([
        parts[0].as_str(),
        parts[1].as_str(),
        &uuid::Uuid::new_v4().to_string(),
    ])
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

// ── Channel ───────────────────────────────────────────────────────────────────

/// Client-side channel for making RPC calls.
///
/// Manages a single persistent SLIM session: either a `PointToPoint` session
/// to a single remote server, or a `Multicast` (GROUP) session shared across
/// multiple remote servers. The session is lazily initialised and recreated
/// when dead.
///
/// ## Constructor
///
/// - `new_with_members_internal(app, members)` — Smart constructor:
///   - **1 member**: creates a P2P channel.
///   - **Many members**: creates a GROUP channel with a generated session name
///     and auto-invites all members on the first multicast call.
#[derive(Clone, uniffi::Object)]
pub struct Channel {
    app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
    /// Session destination: the remote server name for P2P channels, or a
    /// generated group name (same org/namespace as the client, random UUID)
    /// for GROUP channels.
    remote: Name,
    /// `true` for GROUP channels (`new_group` / `new_group_with_connection`),
    /// `false` for P2P channels (`new` / `new_with_connection`).
    is_group: bool,
    /// Initial group members set at construction time. Used to seed a new
    /// session on the first call. Dynamically invited members are stored
    /// in `ChannelSession::members` and inherited across session recreations.
    initial_members: HashSet<Name>,
    connection_id: Option<u64>,
    runtime: tokio::runtime::Handle,
    /// Persistent session (lazily initialised, recreated when dead).
    /// PointToPoint for P2P channels, Multicast for GROUP channels.
    session: Arc<tokio::sync::Mutex<Option<ChannelSession>>>,
}

impl Channel {
    /// Create a channel. This is the single construction point.
    ///
    /// - `is_group = false`: P2P channel; `members` must contain exactly one entry.
    /// - `is_group = true`: GROUP channel; a random UUID session name is generated
    ///   and all `members` are auto-invited on the first multicast call.
    ///
    /// Returns an error if `members` is empty.
    pub fn new_with_members_internal(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        members: Vec<Name>,
        is_group: bool,
        connection_id: Option<u64>,
    ) -> Result<Self, RpcError> {
        if members.is_empty() {
            return Err(RpcError::invalid_argument("members must not be empty"));
        }
        if !is_group && members.len() != 1 {
            return Err(RpcError::invalid_argument(
                "P2P channel requires exactly one member",
            ));
        }

        let runtime = crate::get_runtime().handle().clone();

        let members_set: HashSet<Name> = members.into_iter().collect();

        let remote = if is_group {
            generate_group_name(app.app_name())
        } else {
            members_set.iter().next().unwrap().clone()
        };

        Ok(Self {
            app,
            remote,
            is_group,
            initial_members: members_set,
            connection_id,
            runtime,
            session: Arc::new(tokio::sync::Mutex::new(None)),
        })
    }

    // ── Session management ────────────────────────────────────────────────────

    /// Return the active session, creating it if needed.
    ///
    /// - Single-member channel (`is_group == false`): uses the P2P slot.
    /// - Multi-member channel (`is_group == true`): uses the GROUP slot and
    ///   auto-invites all members on every (re)creation under the session lock.
    async fn get_or_create_session(
        &self,
    ) -> Result<(SessionTx, Arc<ResponseDispatcher>, usize), RpcError> {
        let mut guard = self.session.lock().await;
        self.ensure_session(&mut guard).await?;
        let cs = guard
            .as_ref()
            .ok_or_else(|| RpcError::internal("session missing after creation"))?;
        Ok((cs.tx.clone(), cs.dispatcher.clone(), cs.members.len()))
    }

    /// Create (or recreate) the SLIM session inside an already-held lock guard.
    ///
    /// If the session is already alive this is a no-op. Otherwise the old
    /// (dead) session's member set is inherited so dynamically invited members
    /// survive reconnects. All members are invited on every creation.
    async fn ensure_session(
        &self,
        guard: &mut tokio::sync::MutexGuard<'_, Option<ChannelSession>>,
    ) -> Result<(), RpcError> {
        if let Some(ref cs) = **guard
            && cs.is_alive()
        {
            return Ok(());
        }

        let session_type = if self.is_group {
            ProtoSessionType::Multicast
        } else {
            ProtoSessionType::PointToPoint
        };

        // Preserve members from the old (dead) session so dynamically invited
        // members survive session recreations. Fall back to initial_members on
        // first creation.
        let members = guard
            .take()
            .map(|old| old.members)
            .unwrap_or_else(|| self.initial_members.clone());

        tracing::debug!(?session_type, "no persistent session — recreating");
        let (session_tx, session_rx) = self.create_raw_session_typed(session_type).await?;
        let dispatcher = Arc::new(ResponseDispatcher::new());
        let task = tokio::spawn(response_dispatcher_task(session_rx, dispatcher.clone()));

        **guard = Some(ChannelSession {
            tx: session_tx.clone(),
            dispatcher: dispatcher.clone(),
            task,
            members,
        });

        // Invite all members whenever the GROUP session is (re)created.
        if self.is_group
            && let Some(ref cs) = **guard
        {
            for member in &cs.members {
                send_invite(&session_tx, member).await?;
            }
        }

        Ok(())
    }

    /// Invite a participant into the GROUP session.
    ///
    /// Creates the GROUP session if not yet established. Checks for duplicates
    /// before any session work, then invites and inserts
    ///
    /// Returns an error if called on a P2P channel or if `destination` is
    /// already a member.
    pub async fn invite_participant(&self, destination: Name) -> Result<(), RpcError> {
        if !self.is_group {
            return Err(RpcError::invalid_argument(
                "invite_participant is only valid on GROUP channels",
            ));
        }

        let mut guard = self.session.lock().await;

        // Check for duplicates before doing any work. A dead session still
        // carries its member set (inherited on recreation), so checking it is
        // correct regardless of liveness. Fall back to initial_members when no
        // session exists yet.
        let already_member = match guard.as_ref() {
            Some(s) => s.members.contains(&destination),
            None => self.initial_members.contains(&destination),
        };
        if already_member {
            return Err(RpcError::already_exists(format!(
                "{} is already a member",
                destination
            )));
        }

        // Create the session if needed — reuses the lock already held.
        self.ensure_session(&mut guard).await?;

        let session = guard
            .as_mut()
            .ok_or_else(|| RpcError::internal("session disappeared"))?;
        send_invite(&session.tx, &destination).await?;
        session.members.insert(destination);
        Ok(())
    }

    /// Create a raw SLIM session to the remote peer.
    async fn create_raw_session_typed(
        &self,
        session_type: ProtoSessionType,
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

            let slim_config = slim_session::session_config::SessionConfig {
                session_type,
                mls_enabled: true,
                max_retries: Some(10),
                interval: Some(Duration::from_secs(1)),
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

    /// Send a single request message.
    async fn send_request_impl<Req>(
        &self,
        session: &SessionTx,
        ctx: &Context,
        request: Req,
        service_name: &str,
        method_name: &str,
        rpc_id: &str,
    ) -> Result<(), RpcError>
    where
        Req: Encoder,
    {
        let request_bytes = request.encode()?;
        let meta = first_msg_metadata(ctx, service_name, method_name, rpc_id);
        let handle = session
            .publish(
                session.destination(),
                request_bytes,
                Some("msg".to_string()),
                Some(meta),
            )
            .await?;
        tracing::debug!(%service_name, %method_name, %rpc_id, "Sent request");
        handle.await.map_err(|e| {
            RpcError::internal(format!(
                "Failed to complete sending {}-{}: {}",
                service_name,
                method_name,
                e.chain()
            ))
        })?;
        Ok(())
    }

    /// Send a stream of request messages.
    async fn send_request_stream_impl<Req>(
        &self,
        session: &SessionTx,
        ctx: &Context,
        request_stream: impl Stream<Item = Req> + Send + 'static,
        service_name: &str,
        method_name: &str,
        rpc_id: &str,
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
                first_msg_metadata(ctx, service_name, method_name, rpc_id)
            } else {
                continuation_metadata(rpc_id)
            };
            let handle = session
                .publish(
                    session.destination(),
                    request_bytes,
                    Some("msg".to_string()),
                    Some(msg_meta),
                )
                .await?;
            handles.push(handle);
        }
        let routing = if first {
            Some((ctx, service_name, method_name))
        } else {
            None
        };
        let handle = session
            .publish(
                session.destination(),
                Vec::new(),
                Some("msg".to_string()),
                Some(eos_metadata(rpc_id, routing)),
            )
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

    /// Core streaming method for single-request patterns (P2P and GROUP).
    ///
    /// Sends `request` once and streams back every response as a `MulticastItem`.
    /// For P2P (`num_members = 1`) the stream ends on the single server EOS.
    /// For GROUP the stream ends when all members have sent their EOS.
    /// Member errors are yielded as `Err` items and count as that member's EOS.
    fn responses_from_request<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request: Req,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> impl Stream<Item = Result<MulticastItem<Res>, RpcError>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        let service_name = service_name.to_string();
        let method_name = method_name.to_string();
        let channel = self.clone();

        stream! {
            let ctx = channel.create_context_for_rpc(timeout, metadata.clone());
            let timeout_duration = calculate_timeout_duration(timeout);
            let mut delay = Delay::new(timeout_duration);

            let (session_tx, dispatcher, num_members) = match tokio::select! {
                result = channel.get_or_create_session() => result,
                _ = &mut delay => Err(RpcError::deadline_exceeded("Client deadline exceeded during session setup")),
            } {
                Ok(v) => v,
                Err(e) => { yield Err(e); return; }
            };

            let rpc_id = generate_rpc_id();
            let mut rx = dispatcher.register(&rpc_id);
            let _guard = DispatcherGuard { dispatcher: dispatcher.clone(), rpc_id: rpc_id.clone() };

            let send_result = tokio::select! {
                result = channel.send_request_impl(
                    &session_tx, &ctx, request,
                    &service_name, &method_name, &rpc_id,
                ) => result,
                _ = &mut delay => Err(RpcError::deadline_exceeded("Client deadline exceeded while sending request")),
            };
            if let Err(e) = send_result { yield Err(e); return; }
            let mut eos_count = 0usize;

            loop {
                let received_opt = tokio::select! {
                    msg = rx.recv() => Ok(msg),
                    _ = &mut delay => Err(RpcError::deadline_exceeded(
                        "Client deadline exceeded while receiving response"
                    )),
                };

                let received = match received_opt {
                    Err(e) => { yield Err(e); break; }
                    Ok(None) => {
                        if eos_count < num_members {
                            yield Err(RpcError::internal(format!(
                                "Session closed after {}/{} EOS markers",
                                eos_count, num_members
                            )));
                        }
                        break;
                    }
                    Ok(Some(m)) => m,
                };

                if msg_is_terminal(&received) {
                    eos_count += 1;
                    if eos_count >= num_members { break; }
                    continue;
                }

                let code = match channel.parse_status_code(received.metadata.get(STATUS_CODE_KEY)) {
                    Ok(c) => c,
                    Err(e) => { yield Err(e); break; }
                };
                if code != RpcCode::Ok {
                    let msg_text = String::from_utf8_lossy(&received.payload).to_string();
                    eos_count += 1;
                    yield Err(RpcError::new(code, msg_text));
                    if eos_count >= num_members { break; }
                    continue;
                }

                let source = received.source.clone();
                let response = match Res::decode(received.payload) {
                    Ok(r) => r,
                    Err(e) => {
                        eos_count += 1;
                        yield Err(e);
                        if eos_count >= num_members { break; }
                        continue;
                    }
                };
                yield Ok(MulticastItem { context: MessageContext { source }, message: response });
            }
        }
    }

    /// Core streaming method for streaming-request patterns (P2P and GROUP).
    ///
    /// Broadcasts `request_stream` concurrently while collecting responses.
    /// For P2P (`num_members = 1`) the stream ends on the single server EOS.
    /// For GROUP the stream ends when all members have sent their EOS.
    /// Member errors are yielded as `Err` items and count as that member's EOS.
    fn responses_from_stream_input<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request_stream: impl Stream<Item = Req> + Send + 'static,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> impl Stream<Item = Result<MulticastItem<Res>, RpcError>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        let service_name = service_name.to_string();
        let method_name = method_name.to_string();
        let channel = self.clone();

        stream! {
            let ctx = channel.create_context_for_rpc(timeout, metadata.clone());
            let timeout_duration = calculate_timeout_duration(timeout);
            let mut delay = Delay::new(timeout_duration);

            let (session_tx, dispatcher, num_members) = match tokio::select! {
                result = channel.get_or_create_session() => result,
                _ = &mut delay => Err(RpcError::deadline_exceeded("Client deadline exceeded during session setup")),
            } {
                Ok(v) => v,
                Err(e) => { yield Err(e); return; }
            };

            let rpc_id = generate_rpc_id();
            let mut rx = dispatcher.register(&rpc_id);
            let _guard = DispatcherGuard { dispatcher: dispatcher.clone(), rpc_id: rpc_id.clone() };

            let session_tx_for_send = session_tx.clone();
            let ctx_for_send = ctx.clone();
            let service_for_send = service_name.clone();
            let method_for_send = method_name.clone();
            let rpc_id_for_send = rpc_id.clone();
            let channel_for_send = channel.clone();
            let mut send_handle = channel.runtime.spawn(async move {
                channel_for_send
                    .send_request_stream_impl(
                        &session_tx_for_send, &ctx_for_send, request_stream,
                        &service_for_send, &method_for_send, &rpc_id_for_send,
                    )
                    .await
            });
            let mut eos_count = 0usize;
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
                    Err(e) => { yield Err(e); break; }
                    Ok(None) => {
                        if eos_count < num_members {
                            yield Err(RpcError::internal(format!(
                                "Session closed after {}/{} EOS markers",
                                eos_count, num_members
                            )));
                        }
                        break;
                    }
                    Ok(Some(m)) => m,
                };

                if msg_is_terminal(&received) {
                    eos_count += 1;
                    if eos_count >= num_members { break; }
                    continue;
                }

                let code = match channel.parse_status_code(received.metadata.get(STATUS_CODE_KEY)) {
                    Ok(c) => c,
                    Err(e) => { yield Err(e); break; }
                };
                if code != RpcCode::Ok {
                    let msg_text = String::from_utf8_lossy(&received.payload).to_string();
                    eos_count += 1;
                    yield Err(RpcError::new(code, msg_text));
                    if eos_count >= num_members { break; }
                    continue;
                }

                let source = received.source.clone();
                let response = match Res::decode(received.payload) {
                    Ok(r) => r,
                    Err(e) => {
                        eos_count += 1;
                        yield Err(e);
                        if eos_count >= num_members { break; }
                        continue;
                    }
                };
                yield Ok(MulticastItem { context: MessageContext { source }, message: response });
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
            self.responses_from_request(service_name, method_name, request, timeout, metadata);
        futures::pin_mut!(stream);
        stream
            .next()
            .await
            .unwrap_or_else(|| Err(RpcError::internal("No response received")))
            .map(|item| item.message)
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
        self.responses_from_request(service_name, method_name, request, timeout, metadata)
            .map(|r| r.map(|item| item.message))
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
            service_name,
            method_name,
            request_stream,
            timeout,
            metadata,
        );
        futures::pin_mut!(stream);
        stream
            .next()
            .await
            .unwrap_or_else(|| Err(RpcError::internal("No response received")))
            .map(|item| item.message)
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
            service_name,
            method_name,
            request_stream,
            timeout,
            metadata,
        )
        .map(|r| r.map(|item| item.message))
    }

    /// Multicast unary: broadcast one request, receive one `MulticastItem` per member.
    ///
    /// Each item carries a `MessageContext` identifying the source member plus the
    /// decoded response. The stream ends when all members have sent their response.
    pub fn multicast_unary<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request: Req,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> impl Stream<Item = Result<MulticastItem<Res>, RpcError>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        self.responses_from_request(service_name, method_name, request, timeout, metadata)
    }

    /// Multicast unary-stream: broadcast one request, receive a stream of `MulticastItem`s.
    ///
    /// Each group member may return multiple responses. All responses from all
    /// members are interleaved in arrival order, each tagged with its source.
    /// The stream ends when all members have sent their final EOS.
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
    ) -> impl Stream<Item = Result<MulticastItem<Res>, RpcError>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        self.responses_from_request(service_name, method_name, request, timeout, metadata)
    }

    /// Multicast stream-unary: broadcast a request stream, receive one `MulticastItem` per member.
    ///
    /// The request stream is broadcast to all members. Each member replies with
    /// a single response tagged with its source. The stream ends when all members
    /// have sent their response.
    pub fn multicast_stream_unary<Req, Res>(
        &self,
        service_name: &str,
        method_name: &str,
        request_stream: impl Stream<Item = Req> + Send + 'static,
        timeout: Option<Duration>,
        metadata: Option<Metadata>,
    ) -> impl Stream<Item = Result<MulticastItem<Res>, RpcError>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        self.responses_from_stream_input(
            service_name,
            method_name,
            request_stream,
            timeout,
            metadata,
        )
    }

    /// Multicast stream-stream: broadcast a request stream, receive a stream of `MulticastItem`s.
    ///
    /// The request stream is broadcast to all members. Each member replies with
    /// its own response stream. All responses from all members are interleaved in
    /// arrival order, each tagged with its source. The stream ends when all members
    /// have sent their final EOS.
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
    ) -> impl Stream<Item = Result<MulticastItem<Res>, RpcError>>
    where
        Req: Encoder + Send + 'static,
        Res: Decoder + Send + 'static,
    {
        self.responses_from_stream_input(
            service_name,
            method_name,
            request_stream,
            timeout,
            metadata,
        )
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
        Self::new_with_members_internal(slim_app, vec![slim_name], false, connection_id)
            .expect("single non-empty member list is always valid")
    }

    #[uniffi::constructor]
    pub fn new_group(
        app: std::sync::Arc<crate::App>,
        members: Vec<std::sync::Arc<crate::Name>>,
    ) -> Result<Self, RpcError> {
        Self::new_group_with_connection(app, members, None)
    }

    #[uniffi::constructor]
    pub fn new_group_with_connection(
        app: std::sync::Arc<crate::App>,
        members: Vec<std::sync::Arc<crate::Name>>,
        connection_id: Option<u64>,
    ) -> Result<Self, RpcError> {
        let slim_app = app.inner_app().clone();
        let slim_names = members.iter().map(|n| n.as_slim_name().clone()).collect();
        Self::new_with_members_internal(slim_app, slim_names, true, connection_id)
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

    // ── Multicast UniFFI methods ───────────────────────────────────────────────

    /// Broadcast one request to all GROUP members and collect their responses.
    ///
    /// Returns a reader from which each member's response (wrapped in
    /// `MulticastStreamMessage`) can be pulled one at a time (blocking).
    pub fn call_multicast_unary(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<std::sync::Arc<MulticastResponseReader>, RpcError> {
        crate::get_runtime().block_on(self.call_multicast_unary_async(
            service_name,
            method_name,
            request,
            timeout,
            metadata,
        ))
    }

    /// Broadcast one request to all GROUP members and collect their responses
    /// (async).
    pub async fn call_multicast_unary_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<std::sync::Arc<MulticastResponseReader>, RpcError> {
        let channel = self.clone();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        crate::get_runtime().spawn(async move {
            let stream = channel.multicast_unary::<Vec<u8>, Vec<u8>>(
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
        Ok(std::sync::Arc::new(MulticastResponseReader::new(rx)))
    }

    /// Broadcast one request to all GROUP members and stream their responses
    /// (blocking).
    ///
    /// Semantically identical to `call_multicast_unary` at the transport level;
    /// the difference is that each member may send multiple responses before its
    /// EOS, which the server handler determines.
    pub fn call_multicast_unary_stream(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<std::sync::Arc<MulticastResponseReader>, RpcError> {
        crate::get_runtime().block_on(self.call_multicast_unary_stream_async(
            service_name,
            method_name,
            request,
            timeout,
            metadata,
        ))
    }

    /// Broadcast one request to all GROUP members and stream their responses
    /// (async).
    pub async fn call_multicast_unary_stream_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> Result<std::sync::Arc<MulticastResponseReader>, RpcError> {
        let channel = self.clone();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        crate::get_runtime().spawn(async move {
            let stream = channel.multicast_unary_stream::<Vec<u8>, Vec<u8>>(
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
        Ok(std::sync::Arc::new(MulticastResponseReader::new(rx)))
    }

    /// Broadcast a request stream to all GROUP members and collect their
    /// responses.
    ///
    /// Returns a handler that lets you send requests and receive responses
    /// concurrently. Use `send` / `send_async` to push request messages,
    /// `close_send` / `close_send_async` to signal end-of-requests, and
    /// `recv` / `recv_async` to pull response items.
    pub fn call_multicast_stream_unary(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> std::sync::Arc<MulticastBidiStreamHandler> {
        std::sync::Arc::new(MulticastBidiStreamHandler::new(
            self.clone(),
            service_name,
            method_name,
            timeout,
            metadata,
        ))
    }

    /// Broadcast a request stream to all GROUP members and stream their
    /// responses.
    ///
    /// Semantically equivalent to `call_multicast_stream_unary` at the
    /// transport level; the difference (one vs many responses per member) is
    /// determined by the server handler.
    pub fn call_multicast_stream_stream(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
        metadata: Option<Metadata>,
    ) -> std::sync::Arc<MulticastBidiStreamHandler> {
        std::sync::Arc::new(MulticastBidiStreamHandler::new(
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

    // ── ResponseDispatcher ────────────────────────────────────────────────────

    #[test]
    fn test_response_dispatcher_close_all() {
        let dispatcher = ResponseDispatcher::new();
        let mut rx1 = dispatcher.register("rpc-a");
        let mut rx2 = dispatcher.register("rpc-b");

        dispatcher.close_all();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            assert!(
                rx1.recv().await.is_none(),
                "rx1 should see None after close_all"
            );
            assert!(
                rx2.recv().await.is_none(),
                "rx2 should see None after close_all"
            );
        });
    }

    #[test]
    fn test_response_dispatcher_dispatch_and_unregister() {
        let dispatcher = ResponseDispatcher::new();
        let mut rx = dispatcher.register("rpc-x");

        let dummy_name = Name::from_strings(["", "", ""]);
        let msg = ReceivedMessage {
            metadata: HashMap::new(),
            payload: vec![1, 2, 3],
            source: dummy_name.clone(),
        };
        assert!(dispatcher.dispatch(msg, "rpc-x"));
        assert!(!dispatcher.dispatch(
            ReceivedMessage {
                metadata: HashMap::new(),
                payload: vec![],
                source: dummy_name.clone(),
            },
            "rpc-unknown"
        ));

        dispatcher.unregister("rpc-x");
        assert!(!dispatcher.dispatch(
            ReceivedMessage {
                metadata: HashMap::new(),
                payload: vec![],
                source: dummy_name.clone(),
            },
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
            ReceivedMessage {
                metadata: HashMap::new(),
                payload: vec![],
                source: Name::from_strings(["", "", ""]),
            },
            "rpc-guard"
        ));
    }
}
