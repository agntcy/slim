// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! WASM/Browser entry point for the SLIM data plane.
//!
//! Provides a JavaScript-callable API via `wasm-bindgen` for connecting to a
//! SLIM data-plane instance from a web browser over WebSocket.
//!
//! ## Architecture
//!
//! Unlike the native data plane (which runs an in-process forwarder serving
//! many local apps and many remote peers), the browser is *just an app*: it
//! drives a single embedded `MessageProcessor` whose only purpose is to
//! multiplex one local connection (the JS-facing app) onto one or more
//! outgoing WebSocket connections to remote SLIM nodes. The session layer
//! talks to the embedded data plane through the same `(tx_slim, rx_slim)`
//! channel pair used on native, so subscription routing, link negotiation,
//! and remote subscription acks all just work.
//!
//! ## Quick Start (JavaScript)
//!
//! ```js
//! import init, { initTracing, SlimClient } from "slim_wasm";
//!
//! await init();
//! initTracing();
//!
//! const client = await SlimClient.connect(
//!   "ws://localhost:46357", "my-shared-secret-at-least-32-bytes!!", "org", "ns", "app"
//! );
//!
//! // Subscribe to receive messages addressed to this name
//! await client.subscribe("org", "ns", "app");
//!
//! // Start listening for events (messages, new sessions)
//! client.listen(
//!   (msg) => console.log("message:", msg),
//!   (session) => console.log("new session:", session),
//! );
//!
//! // Create a session and publish
//! const sessionId = await client.createSession("org", "ns", "remote-app", "point-to-point");
//! await client.publish(sessionId, new TextEncoder().encode("hello"), "text/plain");
//!
//! // Connect to additional SLIM nodes — the browser data plane will route
//! // messages by destination Name across all active connections.
//! const conn2 = await client.addConnection("ws://other-slim:46357");
//! await client.subscribe("org", "ns", "another-name", conn2);
//! ```

use wasm_bindgen::prelude::*;

/// Initialize SLIM tracing (console-based logging for browser dev tools).
/// Call this once before using other SLIM APIs.
#[wasm_bindgen(js_name = "initTracing")]
pub fn init_tracing() {
    console_error_panic_hook::set_once();

    use slim_tracing::TracingConfiguration;
    let config = TracingConfiguration::default();
    let _ = config.setup_tracing_subscriber();
}

// ── WASM-only implementation ──

#[cfg(target_arch = "wasm32")]
mod wasm_impl {
    use std::collections::HashMap;
    use std::sync::Arc;

    use wasm_bindgen::prelude::*;

    use slim_auth::shared_secret::SharedSecret;
    use slim_auth::traits::TokenProvider;
    use slim_datapath::api::{MessageType, ProtoMessage, ProtoSessionMessageType, ProtoSessionType};
    use slim_datapath::message_processing::MessageProcessor;
    use slim_datapath::messages::Name;
    use slim_datapath::messages::utils::SlimHeaderFlags;
    use slim_datapath::runtime::CancellationToken;
    use slim_session::interceptor::{IdentityInterceptor, SessionInterceptorProvider};
    use slim_session::notification::Notification;
    use slim_session::session_controller::SessionController;
    use slim_session::subscription_manager::{SubscriptionManager, SubscriptionOps};
    use slim_session::transmitter::AppTransmitter;
    use slim_session::{Direction, SessionConfig, SessionError, SessionLayer};
    use tokio_with_wasm::sync::mpsc;

    /// Wrapper for `js_sys::Function` that satisfies `Send + Sync` bounds.
    ///
    /// SAFETY: This is only used on `wasm32-unknown-unknown` which is single-threaded.
    /// The `Send`/`Sync` markers exist solely to satisfy trait bounds on `Arc<Mutex<>>`
    /// wrappers required by the session layer infrastructure.
    #[derive(Clone)]
    struct JsCallback(js_sys::Function);
    // SAFETY: WASM is single-threaded; no data races are possible.
    unsafe impl Send for JsCallback {}
    unsafe impl Sync for JsCallback {}

    /// A SLIM client that connects to one or more data-plane instances over
    /// WebSocket. Provides subscribe/unsubscribe, session creation, message
    /// publishing, and event listening for the browser.
    #[wasm_bindgen]
    pub struct SlimClient {
        app_name: Name,
        message_processor: Arc<MessageProcessor>,
        session_layer: Arc<SessionLayer<SharedSecret, SharedSecret, AppTransmitter>>,
        sessions: Arc<parking_lot::Mutex<HashMap<u32, Arc<SessionController>>>>,
        #[allow(clippy::type_complexity)]
        notification_rx:
            Arc<parking_lot::Mutex<Option<mpsc::Receiver<Result<Notification, SessionError>>>>>,
        subscription_manager: SubscriptionManager,
        on_message_cb: Arc<parking_lot::Mutex<Option<JsCallback>>>,
        cancel_token: CancellationToken,
        /// Connection IDs of each remote SLIM node we've opened a websocket to.
        /// Used as the default forward target for `subscribe` when the caller
        /// does not specify a connection explicitly.
        remote_conn_ids: Arc<parking_lot::Mutex<Vec<u64>>>,
        /// JWT-style token derived from the shared secret. Reused for each
        /// new outgoing websocket connection (sent as a `?token=` query
        /// parameter, matching the legacy slim-wasm contract).
        auth_token: String,
    }

    #[wasm_bindgen]
    impl SlimClient {
        /// Create a SLIM client and connect it to a first SLIM data-plane
        /// instance via WebSocket.
        ///
        /// # Arguments
        /// * `endpoint` - WebSocket URL (e.g. `ws://localhost:46357`)
        /// * `shared_secret` - Shared secret for HMAC authentication (>= 32 bytes)
        /// * `org` - Organization component of the local app name
        /// * `ns` - Namespace component
        /// * `app` - Application component
        #[wasm_bindgen]
        pub async fn connect(
            endpoint: &str,
            shared_secret: &str,
            org: &str,
            ns: &str,
            app: &str,
        ) -> Result<SlimClient, JsError> {
            let client = SlimClient::new_internal(shared_secret, org, ns, app)?;
            client.add_connection(endpoint).await?;
            Ok(client)
        }

        /// Internal constructor: builds the embedded MessageProcessor, the
        /// local connection that backs the App, and the session layer. No
        /// websocket has been opened yet at this point.
        fn new_internal(
            shared_secret: &str,
            org: &str,
            ns: &str,
            app: &str,
        ) -> Result<SlimClient, JsError> {
            // Auth
            let auth = SharedSecret::new(app, shared_secret)
                .map_err(|e| JsError::new(&format!("auth error: {e}")))?;

            // Generate app ID from identity token ID (mirrors native App::new_with_direction)
            let app_id = {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let token_id = auth
                    .get_id()
                    .map_err(|e| JsError::new(&format!("get_id error: {e}")))?;
                let mut hasher = DefaultHasher::new();
                token_id.hash(&mut hasher);
                hasher.finish()
            };
            let app_name = Name::from_strings([org, ns, app]).with_id(app_id);
            let auth_verifier = SharedSecret::new(app, shared_secret)
                .map_err(|e| JsError::new(&format!("auth error: {e}")))?;
            let auth_token = auth
                .get_token()
                .map_err(|e| JsError::new(&format!("token error: {e}")))?;

            // Create the embedded data plane. No recovery TTL: a browser tab
            // doesn't outlive its connections in any meaningful way.
            let message_processor = Arc::new(MessageProcessor::new_with_options(
                format!("slim-wasm/{app_name}"),
                Some(std::time::Duration::ZERO),
            ));

            // Register the App's local connection on the data plane. From the
            // forwarder's perspective, this is the same kind of "local app
            // connection" that slim-service::App uses on native.
            let (conn_id, tx_slim, rx_slim) = message_processor
                .register_local_connection(false)
                .map_err(|e| JsError::new(&format!("register_local_connection error: {e}")))?;

            // Channel used by SessionLayer to deliver session notifications
            // (NewSession, NewMessage) up to the JS callback.
            let (tx_app, notification_rx) = mpsc::channel(64);

            // Transmitter wires session interceptors (e.g. IdentityInterceptor)
            // into outbound messages.
            let transmitter = AppTransmitter {
                slim_tx: tx_slim.clone(),
                app_tx: tx_app.clone(),
                interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
            };
            let identity_interceptor = Arc::new(IdentityInterceptor::new(
                auth.clone(),
                auth_verifier.clone(),
            ));
            transmitter.add_interceptor(identity_interceptor);

            let session_layer = Arc::new(SessionLayer::new(
                app_name.clone(),
                auth.clone(),
                auth_verifier,
                conn_id,
                tx_slim,
                tx_app,
                transmitter,
                Direction::Bidirectional,
                String::new(),
            ));

            let subscription_manager = session_layer.subscription_manager();

            let cancel_token = CancellationToken::new();

            // Start the local-app message loop: read from rx_slim, dispatch
            // SubscriptionAcks to the subscription manager, and forward
            // Publish messages into the SessionLayer. This mirrors
            // `slim_service::App::process_messages` for the native case,
            // including its initial *local-only* self-subscription so the
            // embedded data plane has a route from any inbound publishes
            // (addressed to this app's name) into the App layer.
            spawn_app_message_loop(
                app_name.clone(),
                rx_slim,
                session_layer.clone(),
                subscription_manager.clone(),
                cancel_token.child_token(),
            );

            Ok(SlimClient {
                app_name,
                message_processor,
                session_layer,
                sessions: Arc::new(parking_lot::Mutex::new(HashMap::new())),
                notification_rx: Arc::new(parking_lot::Mutex::new(Some(notification_rx))),
                subscription_manager,
                on_message_cb: Arc::new(parking_lot::Mutex::new(None)),
                cancel_token,
                remote_conn_ids: Arc::new(parking_lot::Mutex::new(Vec::new())),
                auth_token,
            })
        }

        /// Open a WebSocket to an additional SLIM data-plane instance and
        /// register it with the embedded data plane. Returns the new
        /// connection ID, which can be passed to [`subscribe`] or
        /// [`unsubscribe`] to scope a subscription to a specific node.
        ///
        /// Calling `addConnection` multiple times lets a single browser tab
        /// fan out across several SLIM nodes; the embedded data plane
        /// forwards messages by destination Name using the subscription
        /// table — no JS-side routing logic required.
        #[wasm_bindgen(js_name = "addConnection")]
        pub async fn add_connection(&self, endpoint: &str) -> Result<u32, JsError> {
            // Build URL with shared-secret token query param. We do this
            // ourselves rather than via slim_config's auth flow because
            // SharedSecret/JWT auth is not yet exposed through the wasm
            // build of slim-config.
            let mut url = url::Url::parse(endpoint)
                .map_err(|e| JsError::new(&format!("invalid URL: {e}")))?;
            url.query_pairs_mut().append_pair("token", &self.auth_token);

            let ws = gloo_net::websocket::futures::WebSocket::open(url.as_str())
                .map_err(|e| JsError::new(&format!("websocket error: {e}")))?;

            let (_handle, conn_id) = self
                .message_processor
                .register_websocket(ws, None)
                .map_err(|e| JsError::new(&format!("register_websocket error: {e}")))?;

            self.remote_conn_ids.lock().push(conn_id);

            tracing::info!(
                %endpoint, %conn_id,
                "registered new remote SLIM websocket connection",
            );

            Ok(conn_id as u32)
        }

        /// Subscribe to receive messages for the given name (`org`/`ns`/`name`).
        ///
        /// If `conn_id` is provided, the subscription is forwarded to that
        /// specific remote SLIM connection. If `conn_id` is `None`, the
        /// most recently added remote connection is used (matching the
        /// single-connection legacy behavior).
        #[wasm_bindgen]
        pub async fn subscribe(
            &self,
            org: &str,
            ns: &str,
            name: &str,
            conn_id: Option<u32>,
        ) -> Result<(), JsError> {
            let sub_name = Name::from_strings([org, ns, name]).with_id(self.session_layer.app_id());

            let target_conn = self.resolve_conn(conn_id);

            let (subscription_id, ack_rx) = self
                .subscription_manager
                .subscribe(&self.app_name, &sub_name, target_conn)
                .await
                .map_err(|e| JsError::new(&format!("subscribe error: {e}")))?;

            SubscriptionManager::await_ack(ack_rx)
                .await
                .map_err(|e| JsError::new(&format!("subscription rejected: {e}")))?;

            self.session_layer.add_app_name(sub_name, subscription_id);
            tracing::info!("subscribed to {org}/{ns}/{name} (conn={target_conn:?})");
            Ok(())
        }

        /// Unsubscribe from a name.
        #[wasm_bindgen]
        pub async fn unsubscribe(
            &self,
            org: &str,
            ns: &str,
            name: &str,
            conn_id: Option<u32>,
        ) -> Result<(), JsError> {
            let unsub_name =
                Name::from_strings([org, ns, name]).with_id(self.session_layer.app_id());

            let subscription_id = self.session_layer.remove_app_name(&unsub_name).unwrap_or(0);
            let target_conn = self.resolve_conn(conn_id);

            let ack_rx = self
                .subscription_manager
                .unsubscribe(&self.app_name, &unsub_name, subscription_id, target_conn)
                .await
                .map_err(|e| JsError::new(&format!("unsubscribe error: {e}")))?;

            SubscriptionManager::await_ack(ack_rx)
                .await
                .map_err(|e| JsError::new(&format!("unsubscription rejected: {e}")))?;

            tracing::info!("unsubscribed from {org}/{ns}/{name}");
            Ok(())
        }

        /// Pick a default forwarding target when the caller hasn't specified
        /// one: the most recently added remote connection.
        fn resolve_conn(&self, conn_id: Option<u32>) -> Option<u64> {
            match conn_id {
                Some(id) => Some(id as u64),
                None => self.remote_conn_ids.lock().last().copied(),
            }
        }

        /// Install a forwarding route in the embedded data plane: tells the
        /// browser-side data plane that messages destined for `org/ns/name`
        /// should be forwarded onto a specific remote SLIM connection.
        ///
        /// Without a route, the local data plane has no entry for arbitrary
        /// remote names so a publish (e.g. an `inviteParticipant` discovery
        /// request, or a `publish` to a peer the browser hasn't subscribed
        /// to) returns `NoMatch` and never reaches the wire.
        ///
        /// Mirrors `slim_service::App::set_route` on native: emits a
        /// `Subscribe` with `recv_from=conn_id`, which registers the
        /// destination at that connection in the local subscription table.
        /// If `conn_id` is omitted, the most recently added remote
        /// connection is used.
        #[wasm_bindgen(js_name = "setRoute")]
        pub async fn set_route(
            &self,
            org: &str,
            ns: &str,
            name: &str,
            conn_id: Option<u32>,
        ) -> Result<(), JsError> {
            let target = self
                .resolve_conn(conn_id)
                .ok_or_else(|| JsError::new("no remote connection available; call addConnection first"))?;
            let dest = Name::from_strings([org, ns, name]);
            let mut msg = ProtoMessage::builder()
                .source(self.app_name.clone())
                .destination(dest)
                .flags(SlimHeaderFlags::default().with_recv_from(target))
                .build_subscribe()
                .map_err(|e| JsError::new(&format!("set_route build error: {e}")))?;

            let identity = self
                .session_layer
                .get_identity_token()
                .map_err(|e| JsError::new(&format!("identity error: {e}")))?;
            msg.get_slim_header_mut().set_identity(identity);

            self.session_layer
                .tx_slim()
                .send(Ok(msg))
                .await
                .map_err(|e| JsError::new(&format!("set_route send error: {e}")))?;

            tracing::info!("set route to {org}/{ns}/{name} via conn={target}");
            Ok(())
        }

        /// Remove a previously-installed forwarding route. Mirrors
        /// `slim_service::App::remove_route` on native (sends an
        /// `Unsubscribe` with `recv_from=conn_id`).
        #[wasm_bindgen(js_name = "removeRoute")]
        pub async fn remove_route(
            &self,
            org: &str,
            ns: &str,
            name: &str,
            conn_id: Option<u32>,
        ) -> Result<(), JsError> {
            let target = self
                .resolve_conn(conn_id)
                .ok_or_else(|| JsError::new("no remote connection available"))?;
            let dest = Name::from_strings([org, ns, name]);
            let mut msg = ProtoMessage::builder()
                .source(self.app_name.clone())
                .destination(dest)
                .flags(SlimHeaderFlags::default().with_recv_from(target))
                .build_unsubscribe()
                .map_err(|e| JsError::new(&format!("remove_route build error: {e}")))?;

            let identity = self
                .session_layer
                .get_identity_token()
                .map_err(|e| JsError::new(&format!("identity error: {e}")))?;
            msg.get_slim_header_mut().set_identity(identity);

            self.session_layer
                .tx_slim()
                .send(Ok(msg))
                .await
                .map_err(|e| JsError::new(&format!("remove_route send error: {e}")))?;

            tracing::info!("removed route to {org}/{ns}/{name} via conn={target}");
            Ok(())
        }

        /// Create a session to a remote application.
        ///
        /// `session_type` must be `"point-to-point"` (or `"p2p"`) or `"multicast"`.
        /// Set `mls_enabled` to `true` for end-to-end encrypted group sessions (MLS).
        /// Returns the session ID on success.
        #[wasm_bindgen(js_name = "createSession")]
        pub async fn create_session(
            &self,
            dest_org: &str,
            dest_ns: &str,
            dest_app: &str,
            session_type: &str,
            mls_enabled: Option<bool>,
        ) -> Result<u32, JsError> {
            let destination = Name::from_strings([dest_org, dest_ns, dest_app]);

            let proto_session_type = match session_type {
                "point-to-point" | "p2p" => ProtoSessionType::PointToPoint,
                "multicast" => ProtoSessionType::Multicast,
                _ => {
                    return Err(JsError::new(&format!(
                        "unknown session type: {session_type}. Use 'point-to-point' or 'multicast'"
                    )));
                }
            };

            let config = SessionConfig {
                session_type: proto_session_type,
                max_retries: Some(10),
                interval: Some(std::time::Duration::from_secs(2)),
                mls_enabled: mls_enabled.unwrap_or(false),
                initiator: true,
                metadata: HashMap::new(),
            };

            let (session_ctx, init_ack) = self
                .session_layer
                .create_session(config, self.app_name.clone(), destination, None)
                .await
                .map_err(|e| JsError::new(&format!("create session error: {e}")))?;

            let session_id = session_ctx.session_id();
            let session_arc = session_ctx
                .session_arc()
                .ok_or_else(|| JsError::new("session already dropped"))?;

            self.sessions.lock().insert(session_id, session_arc.clone());

            // Spawn a receiver to forward session messages to the JS callback
            let on_msg = self.on_message_cb.clone();
            let (_, mut rx) = session_ctx.into_parts();
            let sid = session_id;
            wasm_bindgen_futures::spawn_local(async move {
                tracing::info!(session_id = sid, "session receiver started");
                while let Some(msg_result) = rx.recv().await {
                    match msg_result {
                        Ok(msg) => {
                            tracing::info!(
                                session_id = sid,
                                "session receiver got message, invoking JS callback",
                            );
                            let cb = on_msg.lock().clone();
                            if let Some(callback) = cb {
                                let obj = build_message_js_object(&msg);
                                if let Err(e) = callback.0.call1(&JsValue::NULL, &obj) {
                                    tracing::error!(
                                        session_id = sid,
                                        error = ?e,
                                        "JS on_message callback threw",
                                    );
                                }
                            } else {
                                tracing::warn!(
                                    session_id = sid,
                                    "no on_message callback registered",
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                session_id = sid,
                                error = ?e,
                                "session receiver got error",
                            );
                        }
                    }
                }
                tracing::info!(session_id = sid, "session receiver ended");
            });

            // Wait for session establishment (discovery handshake) with timeout
            tracing::info!(session_id, "waiting for session init_ack (30s timeout)");
            let init_result: Result<Result<(), slim_session::SessionError>, _> = tokio_with_wasm::select! {
                result = init_ack => Ok(result),
                _ = tokio_with_wasm::time::sleep(std::time::Duration::from_secs(30)) => {
                    Err("timeout")
                }
            };
            match init_result {
                Ok(inner) => {
                    inner.map_err(|e| JsError::new(&format!("session init error: {e}")))?;
                }
                Err(_) => {
                    return Err(JsError::new(&format!(
                        "session {session_id} init timed out after 30s"
                    )));
                }
            }

            tracing::info!("session {session_id} created");
            Ok(session_id)
        }

        /// Publish a message through an established session.
        ///
        /// # Arguments
        /// * `session_id` - Session ID returned by `createSession()`
        /// * `payload` - Message payload bytes
        /// * `payload_type` - Optional MIME type (defaults to `"msg"`)
        #[wasm_bindgen]
        pub async fn publish(
            &self,
            session_id: u32,
            payload: &[u8],
            payload_type: Option<String>,
        ) -> Result<(), JsError> {
            let session = self
                .sessions
                .lock()
                .get(&session_id)
                .cloned()
                .ok_or_else(|| JsError::new(&format!("session {session_id} not found")))?;

            let dest = session.dst().clone();
            let ack = session
                .publish(&dest, payload.to_vec(), payload_type, None)
                .await
                .map_err(|e| JsError::new(&format!("publish error: {e}")))?;

            ack.await
                .map_err(|e| JsError::new(&format!("publish ack error: {e}")))?;

            Ok(())
        }

        /// Start listening for incoming messages and session notifications.
        ///
        /// `on_message` is called with a JS object:
        /// ```js
        /// { source: string, payload: Uint8Array, payloadType: string, sessionId: number }
        /// ```
        ///
        /// `on_session` is called with a JS object:
        /// ```js
        /// { sessionId: number, source: string, destination: string }
        /// ```
        ///
        /// Can only be called once. Call before `createSession()` to receive
        /// messages on sessions created by the remote side.
        #[wasm_bindgen]
        pub fn listen(
            &self,
            on_message: js_sys::Function,
            on_session: js_sys::Function,
        ) -> Result<(), JsError> {
            // Store callback for use by session receivers
            *self.on_message_cb.lock() = Some(JsCallback(on_message.clone()));

            let mut rx = self
                .notification_rx
                .lock()
                .take()
                .ok_or_else(|| JsError::new("listen() has already been called"))?;

            let sessions = self.sessions.clone();
            let on_msg_store = self.on_message_cb.clone();
            let cancel = self.cancel_token.child_token();
            let on_message = JsCallback(on_message);
            let on_session = JsCallback(on_session);

            wasm_bindgen_futures::spawn_local(async move {
                loop {
                    let next = tokio_with_wasm::select! {
                        m = rx.recv() => m,
                        _ = cancel.cancelled() => None,
                    };
                    match next {
                        None => break,
                        Some(Ok(notification)) => match notification {
                            Notification::NewMessage(msg) => {
                                let obj = build_message_js_object(&msg);
                                on_message.0.call1(&JsValue::NULL, &obj).ok();
                            }
                            Notification::NewSession(ctx) => {
                                let session_id = ctx.session_id();
                                let session_arc = ctx.session_arc();

                                // Build JS notification
                                let obj = js_sys::Object::new();
                                js_sys::Reflect::set(
                                    &obj,
                                    &"sessionId".into(),
                                    &JsValue::from(session_id),
                                )
                                .ok();

                                if let Some(ref sa) = session_arc {
                                    let src = format!("{}", sa.source());
                                    let dst = format!("{}", sa.dst());
                                    js_sys::Reflect::set(&obj, &"source".into(), &src.into()).ok();
                                    js_sys::Reflect::set(&obj, &"destination".into(), &dst.into())
                                        .ok();
                                    sessions.lock().insert(session_id, sa.clone());
                                }

                                // Spawn receiver for incoming session messages
                                let on_msg_clone = on_msg_store.clone();
                                let (_, mut srx) = ctx.into_parts();
                                wasm_bindgen_futures::spawn_local(async move {
                                    while let Some(msg_result) = srx.recv().await {
                                        if let Ok(msg) = msg_result {
                                            let cb = on_msg_clone.lock().clone();
                                            if let Some(callback) = cb {
                                                let obj = build_message_js_object(&msg);
                                                callback.0.call1(&JsValue::NULL, &obj).ok();
                                            }
                                        }
                                    }
                                });

                                on_session.0.call1(&JsValue::NULL, &obj.into()).ok();
                            }
                        },
                        Some(Err(e)) => {
                            tracing::warn!("notification error: {e}");
                        }
                    }
                }
                tracing::info!("notification loop ended");
            });

            Ok(())
        }

        /// Invite a participant to a multicast (group) session.
        ///
        /// Only the session initiator (moderator) can invite participants.
        /// The participant must be subscribed to the given name.
        #[wasm_bindgen(js_name = "inviteParticipant")]
        pub async fn invite_participant(
            &self,
            session_id: u32,
            org: &str,
            ns: &str,
            name: &str,
        ) -> Result<(), JsError> {
            let session = self
                .sessions
                .lock()
                .get(&session_id)
                .cloned()
                .ok_or_else(|| JsError::new(&format!("session {session_id} not found")))?;

            let dest = Name::from_strings([org, ns, name]);
            let ack = session
                .invite_participant(&dest)
                .await
                .map_err(|e| JsError::new(&format!("invite error: {e}")))?;

            ack.await
                .map_err(|e| JsError::new(&format!("invite ack error: {e}")))?;

            tracing::info!("invited {org}/{ns}/{name} to session {session_id}");
            Ok(())
        }

        /// Remove a participant from a multicast (group) session.
        ///
        /// Only the session initiator (moderator) can remove participants.
        #[wasm_bindgen(js_name = "removeParticipant")]
        pub async fn remove_participant(
            &self,
            session_id: u32,
            org: &str,
            ns: &str,
            name: &str,
        ) -> Result<(), JsError> {
            let session = self
                .sessions
                .lock()
                .get(&session_id)
                .cloned()
                .ok_or_else(|| JsError::new(&format!("session {session_id} not found")))?;

            let dest = Name::from_strings([org, ns, name]);
            let ack = session
                .remove_participant(&dest)
                .await
                .map_err(|e| JsError::new(&format!("remove error: {e}")))?;

            ack.await
                .map_err(|e| JsError::new(&format!("remove ack error: {e}")))?;

            tracing::info!("removed {org}/{ns}/{name} from session {session_id}");
            Ok(())
        }

        /// Get the list of participants in a session.
        #[wasm_bindgen(js_name = "participantsList")]
        pub async fn participants_list(&self, session_id: u32) -> Result<Vec<String>, JsError> {
            let session = self
                .sessions
                .lock()
                .get(&session_id)
                .cloned()
                .ok_or_else(|| JsError::new(&format!("session {session_id} not found")))?;

            let names = session
                .participants_list()
                .await
                .map_err(|e| JsError::new(&format!("participants list error: {e}")))?;

            Ok(names.into_iter().map(|n| format!("{n}")).collect())
        }

        /// Delete a session by ID.
        #[wasm_bindgen(js_name = "deleteSession")]
        pub fn delete_session(&self, session_id: u32) -> Result<(), JsError> {
            self.sessions.lock().remove(&session_id);
            let _ = self.session_layer.remove_session(session_id);
            tracing::info!("session {session_id} deleted");
            Ok(())
        }

        /// Disconnect from all SLIM data planes by cancelling background tasks.
        /// The data plane closes each registered websocket cleanly via its
        /// per-connection cancellation token.
        #[wasm_bindgen]
        pub fn disconnect(&self) {
            self.cancel_token.cancel();
            for conn_id in self.remote_conn_ids.lock().drain(..) {
                let _ = self.message_processor.disconnect(conn_id);
            }
            tracing::info!("disconnected from SLIM");
        }

        /// Get the local app name.
        #[wasm_bindgen(getter, js_name = "appName")]
        pub fn app_name(&self) -> String {
            format!("{}", self.app_name)
        }

        /// Get IDs of all active sessions.
        #[wasm_bindgen(js_name = "sessionIds")]
        pub fn session_ids(&self) -> Vec<u32> {
            self.sessions.lock().keys().copied().collect()
        }

        /// Get the connection IDs of all currently registered remote
        /// SLIM nodes (in the order they were added).
        #[wasm_bindgen(js_name = "connectionIds")]
        pub fn connection_ids(&self) -> Vec<u32> {
            self.remote_conn_ids
                .lock()
                .iter()
                .map(|&id| id as u32)
                .collect()
        }
    }

    /// Pump messages from the data plane → local app and dispatch them to
    /// the SessionLayer / SubscriptionManager. Equivalent to
    /// `slim_service::App::process_messages` for the native build.
    ///
    /// Mirrors the native loop's startup behaviour: before entering the
    /// dispatch loop we kick off a *local-only* self-subscription
    /// (`subscribe(app_name, app_name, None)` — no `forward_to`) so the
    /// embedded data plane registers a route from this name to the local
    /// app connection. That route is what lets inbound publishes from the
    /// gateway eventually land in `rx_slim` and surface to the App layer.
    /// It does *not* register the name on the gateway — the user is still
    /// expected to call `SlimClient::subscribe(...)` for that, exactly as
    /// `sdk-mock` calls `app.subscribe(app.app_name(), Some(conn_id))`
    /// after `Service::connect`.
    fn spawn_app_message_loop(
        app_name: Name,
        mut rx: mpsc::Receiver<Result<ProtoMessage, slim_datapath::Status>>,
        session_layer: Arc<SessionLayer<SharedSecret, SharedSecret, AppTransmitter>>,
        subscription_manager: SubscriptionManager,
        cancel: CancellationToken,
    ) {
        wasm_bindgen_futures::spawn_local(async move {
            // Initiate the local self-subscription so the ACK is tracked
            // and resolved through the normal loop machinery below. This
            // mirrors `slim_service::App::process_messages`'s startup.
            let (_init_sub_id, init_ack_rx) = subscription_manager
                .subscribe(&app_name, &app_name, None)
                .await
                .expect("error sending initial self-subscription");
            let mut init_ack_future = std::pin::pin!(init_ack_rx);
            let mut init_ack_done = false;

            loop {
                tokio_with_wasm::select! {
                    next = rx.recv() => {
                        match next {
                            None => {
                                tracing::info!("app message loop ended (slim channel closed)");
                                break;
                            }
                            Some(Ok(msg)) => {
                                match msg.message_type.as_ref() {
                                    Some(MessageType::Publish(_)) => {
                                        if let Err(e) =
                                            session_layer.handle_message_from_slim(msg).await
                                            && !matches!(e, SessionError::SubscriptionNotFound(_))
                                        {
                                            tracing::warn!(error = ?e, "handle_message_from_slim error");
                                        }
                                    }
                                    Some(MessageType::SubscriptionAck(ack)) => {
                                        subscription_manager.resolve_ack(ack);
                                    }
                                    // Subscribe / Unsubscribe / Link / None: not
                                    // expected on the app-facing channel; the data
                                    // plane's `process_stream` already handled them.
                                    _ => {}
                                }
                            }
                            Some(Err(e)) => {
                                tracing::warn!(error = %e, "received error from SLIM");
                            }
                        }
                    }
                    result = &mut init_ack_future, if !init_ack_done => {
                        init_ack_done = true;
                        match result {
                            Ok(Ok(())) => tracing::debug!(%app_name, "initial self-subscription confirmed"),
                            Ok(Err(e)) => tracing::error!(%app_name, error = %e, "initial self-subscription failed"),
                            Err(_) => tracing::error!(%app_name, "initial self-subscription ack channel closed"),
                        }
                    }
                    _ = cancel.cancelled() => {
                        break;
                    }
                }
            }
        });
    }

    /// Convert a ProtoMessage into a JS object for the on_message callback.
    fn build_message_js_object(msg: &ProtoMessage) -> JsValue {
        let obj = js_sys::Object::new();

        let source = format!("{}", msg.get_source());
        js_sys::Reflect::set(&obj, &"source".into(), &source.into()).ok();

        let session_id = msg.get_session_header().session_id;
        js_sys::Reflect::set(&obj, &"sessionId".into(), &JsValue::from(session_id)).ok();

        if msg.is_publish() {
            if let Some(content) = msg.get_payload() {
                if let Ok(app_payload) = content.as_application_payload() {
                    let arr = js_sys::Uint8Array::from(&app_payload.blob[..]);
                    js_sys::Reflect::set(&obj, &"payload".into(), &arr).ok();
                    js_sys::Reflect::set(
                        &obj,
                        &"payloadType".into(),
                        &app_payload.payload_type.clone().into(),
                    )
                    .ok();
                }
            }
        }

        // Session message type (Msg, JoinRequest, etc.) — useful for filtering on JS side.
        let smt: ProtoSessionMessageType = msg
            .get_session_header()
            .session_message_type
            .try_into()
            .unwrap_or_default();
        js_sys::Reflect::set(
            &obj,
            &"sessionMessageType".into(),
            &JsValue::from_str(smt.as_str_name()),
        )
        .ok();

        obj.into()
    }
}
