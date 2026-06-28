// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! wasm-bindgen bindings exposing a SLIM **MLS group moderator** to JavaScript.
//!
//! Unlike the previous raw data-plane binding, this client drives the **session
//! layer** (`SessionLayer` + a Multicast session with MLS) directly in the
//! browser. The browser acts as the *moderator*: it creates the encrypted
//! group, invites native participants, and broadcasts end-to-end encrypted
//! messages — all decrypted transparently by the session layer.
//!
//! `agntcy-slim-service` (the native `App`) is native-only, so this module
//! re-implements the small slice of `App` the browser needs (id derivation,
//! the SLIM receive loop, self-subscription, routes) on top of the
//! wasm-capable `agntcy-slim-session` crate.
//!
//! Flow:
//!   * `connect`      — open the WebSocket, run link negotiation, build the
//!     `SessionLayer`, start the receive loop, self-subscribe.
//!   * `create_group` — create the Multicast+MLS session (moderator/initiator).
//!   * `invite`       — route to + invite a native participant into the group.
//!   * `publish`      — broadcast an MLS-encrypted message to the group.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use slim_auth::shared_secret::SharedSecret;
use slim_auth::traits::TokenProvider;
use slim_config::client::ClientConfig;
use slim_datapath::Status;
use slim_datapath::api::proto::dataplane::v1::content::ContentType;
use slim_datapath::api::{MessageType, NameId, ProtoMessage, ProtoName, ProtoSessionType};
use slim_datapath::message_processing::MessageProcessor;
use slim_datapath::messages::utils::SlimHeaderFlags;
use slim_session::session_config::{MlsSettings, SessionConfig};
use slim_session::session_controller::SessionController;
use slim_session::subscription_manager::{SubscriptionManager, SubscriptionOps};
use slim_session::{Direction, Notification, SessionLayer};
use tokio::sync::mpsc::Receiver;
use wasm_bindgen::prelude::*;

/// Shared identity secret. Every participant (browser + native) must use the
/// same secret so MLS credential verification succeeds. Mirrors the native
/// `slim_testing::utils::TEST_VALID_SECRET`.
const TEST_VALID_SECRET: &str = "test-shared-secret-value-0123456789abcdef";

/// Broadcast fanout for group chat: any value > 1 makes the data plane
/// `match_all` (deliver to *every* group member except the sender).
const BROADCAST_FANOUT: u32 = 10;

/// Identity type: a shared secret used as both token provider and verifier.
type Secret = SharedSecret;

/// One-time wasm setup: panic hook + `tracing` → browser console. Safe to call
/// more than once.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    // `set_as_global_default` errors if a subscriber is already installed; the
    // demo only ever sets it once, so ignore the result for idempotency.
    let _ = std::panic::catch_unwind(tracing_wasm::set_as_global_default);
}

/// A connected SLIM browser MLS moderator. Created via [`SlimClient::connect`].
#[wasm_bindgen]
pub struct SlimClient {
    processor: MessageProcessor,
    session_layer: Arc<SessionLayer<Secret, Secret>>,
    subscription_manager: SubscriptionManager,
    /// This moderator's full name (`org/ns/type` with the derived id).
    app_name: ProtoName,
    /// Upstream (Edge) connection id.
    remote_conn: u64,
    /// The active group session + channel name (set by `create_group`).
    session: RefCell<Option<Arc<SessionController>>>,
    channel: RefCell<Option<ProtoName>>,
    /// JS callback `on_message(kind, source, payload)` for inbound group frames.
    on_message: js_sys::Function,
}

#[wasm_bindgen]
impl SlimClient {
    /// Connect to a SLIM server, run link negotiation, build the session layer
    /// and start delivering inbound frames to `on_message`.
    ///
    /// - `endpoint`: `ws://` or `wss://` URL of the SLIM server.
    /// - `token`: optional JWT appended as `?token=<jwt>` for query-string auth.
    /// - `name`: this moderator's name as `org/ns/type` (e.g. `org/default/browser`).
    /// - `on_message`: invoked as `on_message(kind, source, payload)` per inbound
    ///   group frame. `kind`/`source` are strings; `payload` is a `Uint8Array`.
    #[wasm_bindgen]
    pub async fn connect(
        endpoint: String,
        token: Option<String>,
        name: String,
        on_message: js_sys::Function,
    ) -> Result<SlimClient, JsError> {
        let app_base = parse_name(&name)?;
        let url = build_endpoint(&endpoint, token.as_deref());
        tracing::info!("connecting to {url} as '{name}'");

        let config = ClientConfig::with_endpoint(&url);
        let processor = MessageProcessor::new();

        // WebSocket + X25519 link-negotiation handshake.
        let (_handle, remote_conn) = processor
            .connect(config, None, None)
            .await
            .map_err(|e| JsError::new(&format!("connect failed: {e}")))?;
        tracing::info!("upstream connection established (conn {remote_conn})");

        // Local app endpoint: `tx_slim` sends toward SLIM, `rx_slim` receives.
        let (local_conn, tx_slim, rx_slim) = processor
            .register_local_connection(false)
            .map_err(|e| JsError::new(&format!("register local connection failed: {e}")))?;
        tracing::info!("local app connection established (conn {local_conn})");

        // Identity: shared secret as both provider and verifier.
        let identity = SharedSecret::new(&name, TEST_VALID_SECRET)
            .map_err(|e| JsError::new(&format!("failed to create identity: {e}")))?;

        // Derive the app id from the identity token (mirrors native `App`).
        let app_name = derive_app_name(&app_base, &identity)?;
        tracing::info!("app name with id: {app_name}");

        // Notification channel (session -> app). The moderator drains it.
        let (tx_app, rx_app) = tokio::sync::mpsc::channel(128);

        let session_layer = Arc::new(SessionLayer::new(
            app_name.clone(),
            identity.clone(),
            identity,
            remote_conn,
            tx_slim,
            tx_app,
            Direction::Bidirectional,
            "slim/0".to_string(),
        ));
        let subscription_manager = session_layer.subscription_manager();

        // Start the SLIM receive loop (replicates `App::process_messages`).
        spawn_slim_loop(
            session_layer.clone(),
            subscription_manager.clone(),
            app_name.clone(),
            rx_slim,
        );
        // Drain notifications so the bounded channel never blocks the layer.
        spawn_notification_drain(rx_app);

        let client = SlimClient {
            processor,
            session_layer,
            subscription_manager,
            app_name,
            remote_conn,
            session: RefCell::new(None),
            channel: RefCell::new(None),
            on_message,
        };

        // Subscribe to our own name and propagate it upstream so the node routes
        // group traffic addressed to us back down.
        client.self_subscribe().await?;

        Ok(client)
    }

    /// Create the MLS group (Multicast session, moderator/initiator) for channel
    /// `org/ns/name`. Must be called before `invite`/`publish`.
    #[wasm_bindgen]
    pub async fn create_group(
        &self,
        org: String,
        ns: String,
        name: String,
    ) -> Result<(), JsError> {
        let channel = ProtoName::from_strings([org, ns, name]);

        let config = SessionConfig {
            session_type: ProtoSessionType::Multicast,
            max_retries: Some(10),
            interval: Some(Duration::from_secs(1)),
            mls_settings: Some(MlsSettings::default()),
            initiator: true,
            metadata: HashMap::new(),
        };

        let (ctx, completion) = self
            .session_layer
            .create_session(config, self.app_name.clone(), channel.clone(), Some(12345))
            .await
            .map_err(|e| JsError::new(&format!("create_session failed: {e}")))?;

        completion
            .await
            .map_err(|e| JsError::new(&format!("session init failed: {e}")))?;

        let session = ctx
            .session_arc()
            .ok_or_else(|| JsError::new("session was dropped"))?;

        // Drive the session's receive queue on the browser event loop. Wasm
        // futures are `!Send`, so we cannot use `spawn_receiver` (requires Send);
        // we take the raw receiver and pump it via `spawn_local`.
        let (_weak, mut rx) = ctx.into_parts();
        let on_message = self.on_message.clone();
        wasm_bindgen_futures::spawn_local(async move {
            while let Some(item) = rx.recv().await {
                match item {
                    Ok(msg) => deliver(&on_message, &msg),
                    Err(e) => {
                        tracing::error!("session receive error: {e}");
                        break;
                    }
                }
            }
            tracing::info!("group session receiver closed");
        });

        *self.session.borrow_mut() = Some(session);
        *self.channel.borrow_mut() = Some(channel);
        tracing::info!("MLS group created");
        Ok(())
    }

    /// Route to and invite a native participant `org/ns/type` into the group.
    #[wasm_bindgen]
    pub async fn invite(&self, org: String, ns: String, name: String) -> Result<(), JsError> {
        let participant = ProtoName::from_strings([org, ns, name]);

        // Route messages toward the participant via the upstream node.
        self.set_route(&participant).await?;

        let session = self
            .session
            .borrow()
            .clone()
            .ok_or_else(|| JsError::new("no group: call create_group first"))?;

        session
            .invite_participant(&participant)
            .await
            .map_err(|e| JsError::new(&format!("invite failed: {e}")))?;
        tracing::info!("invited participant");
        Ok(())
    }

    /// Broadcast an MLS-encrypted `data` payload to the whole group.
    #[wasm_bindgen]
    pub async fn publish(&self, data: Vec<u8>) -> Result<(), JsError> {
        let session = self
            .session
            .borrow()
            .clone()
            .ok_or_else(|| JsError::new("no group: call create_group first"))?;
        let channel = self
            .channel
            .borrow()
            .clone()
            .ok_or_else(|| JsError::new("no group: call create_group first"))?;

        let flags = SlimHeaderFlags::new(BROADCAST_FANOUT, None, None, None, None);
        session
            .publish_with_flags(&channel, flags, data, None, None)
            .await
            .map_err(|e| JsError::new(&format!("publish failed: {e}")))?;
        Ok(())
    }

    /// Tear down the upstream connection.
    #[wasm_bindgen]
    pub fn close(&self) {
        if let Err(e) = self.processor.disconnect(self.remote_conn) {
            tracing::warn!("disconnect failed: {e}");
        }
    }

    // ── internal helpers ────────────────────────────────────────────────────

    /// Subscribe to our own name and propagate it upstream (mirrors
    /// `App::subscribe(app_name, Some(conn))`).
    async fn self_subscribe(&self) -> Result<(), JsError> {
        let name = self.app_name.clone().with_id(self.session_layer.app_id());
        let (sub_id, ack_rx) = self
            .subscription_manager
            .subscribe(&self.app_name, &name, Some(self.remote_conn))
            .await
            .map_err(|e| JsError::new(&format!("subscribe failed: {e}")))?;

        SubscriptionManager::await_ack(ack_rx)
            .await
            .map_err(|e| JsError::new(&format!("subscribe ack failed: {e}")))?;

        self.session_layer.add_app_name(name, sub_id);
        Ok(())
    }

    /// Add a route toward `name` via the upstream node, with identity attached
    /// (mirrors `App::set_route`).
    async fn set_route(&self, name: &ProtoName) -> Result<(), JsError> {
        let mut msg = ProtoMessage::builder()
            .source(self.app_name.clone())
            .destination(name.clone())
            .flags(SlimHeaderFlags::default().with_recv_from(self.remote_conn))
            .build_subscribe()
            .map_err(|e| JsError::new(&format!("failed to build route: {e}")))?;

        let identity = self
            .session_layer
            .get_identity_token()
            .map_err(|e| JsError::new(&format!("failed to get identity: {e}")))?;
        msg.get_slim_header_mut().set_identity(identity);

        self.session_layer
            .tx_slim()
            .send(Ok(msg))
            .await
            .map_err(|_| JsError::new("client is closed"))
    }
}

/// Derive the moderator's name id from its identity token (mirrors native
/// `App::new_with_direction`: XXH3-128 of the token id, avoiding reserved ids).
fn derive_app_name(base: &ProtoName, identity: &Secret) -> Result<ProtoName, JsError> {
    let token_id = identity
        .get_id()
        .map_err(|e| JsError::new(&format!("failed to get identity id: {e}")))?;
    let mut id_hash = twox_hash::XxHash3_128::oneshot(token_id.as_bytes());
    if NameId::is_reserved_id(id_hash) {
        id_hash %= u128::MAX - NameId::RESERVED_IDS;
    }
    Ok(base.clone().with_id(id_hash))
}

/// SLIM receive loop: replicates `App::process_messages`. Self-subscribes the
/// app name locally, then routes Publish frames into the session layer and
/// resolves subscription acks.
fn spawn_slim_loop(
    session_layer: Arc<SessionLayer<Secret, Secret>>,
    subscription_manager: SubscriptionManager,
    app_name: ProtoName,
    mut rx: Receiver<Result<ProtoMessage, Status>>,
) {
    wasm_bindgen_futures::spawn_local(async move {
        // Initial local self-subscription; its ack is resolved by this loop.
        let _ = subscription_manager
            .subscribe(&app_name, &app_name, None)
            .await;

        while let Some(item) = rx.recv().await {
            match item {
                Ok(msg) => match msg.message_type.as_ref() {
                    Some(MessageType::Publish(_)) => {
                        if let Err(e) = session_layer.handle_message_from_slim(msg).await {
                            match e {
                                slim_session::SessionError::SubscriptionNotFound(_) => {
                                    tracing::debug!("session not found, ignoring message");
                                }
                                _ => tracing::error!("error handling message: {e}"),
                            }
                        }
                    }
                    Some(MessageType::SubscriptionAck(ack)) => {
                        subscription_manager.resolve_ack(ack);
                    }
                    _ => {}
                },
                Err(e) => {
                    tracing::error!("inbound transport error: {e}");
                    break;
                }
            }
        }
        tracing::info!("slim receive loop closed");
    });
}

/// Drain notifications so the bounded channel never blocks the session layer.
fn spawn_notification_drain(mut rx: Receiver<Result<Notification, slim_session::SessionError>>) {
    wasm_bindgen_futures::spawn_local(async move {
        while let Some(item) = rx.recv().await {
            match item {
                Ok(_n) => tracing::debug!("notification received"),
                Err(e) => {
                    tracing::debug!("notification error: {e}");
                    break;
                }
            }
        }
    });
}

/// Invoke the JS `on_message(kind, source, payload)` callback for a frame.
fn deliver(on_message: &js_sys::Function, msg: &ProtoMessage) {
    let kind = JsValue::from_str(message_kind(msg));
    let source = JsValue::from_str(&source_label(msg));
    let payload = js_sys::Uint8Array::from(publish_payload(msg).as_slice());
    if on_message
        .call3(&JsValue::NULL, &kind, &source, &payload.into())
        .is_err()
    {
        tracing::error!("on_message callback threw");
    }
}

/// Parse a 3-component `org/ns/type` name.
fn parse_name(s: &str) -> Result<ProtoName, JsError> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 3 {
        return Err(JsError::new(&format!("name must be 'org/ns/type', got '{s}'")));
    }
    Ok(ProtoName::from_strings([
        parts[0].to_string(),
        parts[1].to_string(),
        parts[2].to_string(),
    ]))
}

fn message_kind(msg: &ProtoMessage) -> &'static str {
    match msg.get_type() {
        MessageType::Publish(_) => "Publish",
        MessageType::Subscribe(_) => "Subscribe",
        MessageType::Unsubscribe(_) => "Unsubscribe",
        MessageType::SubscriptionAck(_) => "SubscriptionAck",
        MessageType::Link(_) => "Link",
    }
}

fn source_label(msg: &ProtoMessage) -> String {
    let name = msg.get_source();
    match &name.str_name {
        Some(s) => format!(
            "{}/{}/{}",
            s.str_component_0, s.str_component_1, s.str_component_2
        ),
        None => "<unknown>".to_string(),
    }
}

/// Application-payload bytes for `Publish` frames; empty for everything else.
/// Avoids the panicking accessors by matching the oneof directly.
fn publish_payload(msg: &ProtoMessage) -> Vec<u8> {
    if let MessageType::Publish(p) = msg.get_type() {
        if let Some(content) = p.msg.as_ref() {
            if let Some(ContentType::AppPayload(app)) = content.content_type.as_ref() {
                return app.blob.clone();
            }
        }
    }
    Vec::new()
}

/// Append `?token=<jwt>` (or `&token=...`) to the endpoint when a token is
/// provided. JWTs are URL-safe (base64url + '.'), so no escaping is needed.
fn build_endpoint(endpoint: &str, token: Option<&str>) -> String {
    match token {
        Some(t) if !t.is_empty() => {
            let sep = if endpoint.contains('?') { '&' } else { '?' };
            format!("{endpoint}{sep}token={t}")
        }
        _ => endpoint.to_string(),
    }
}
