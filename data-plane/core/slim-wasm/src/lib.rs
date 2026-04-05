// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! WASM/Browser entry point for the SLIM data plane.
//!
//! Provides a JavaScript-callable API via `wasm-bindgen` for connecting to a
//! SLIM data-plane instance from a web browser over WebSocket.
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
//! // Group chat (multicast)
//! const groupId = await client.createSession("org", "ns", "channel", "multicast");
//! await client.inviteParticipant(groupId, "org", "ns", "peer-app");
//! await client.publish(groupId, new TextEncoder().encode("hi group!"), "text/plain");
//! const members = await client.participantsList(groupId);
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

    use futures::stream::StreamExt;
    use futures::sink::SinkExt;
    use prost::Message as ProstMessage;
    use wasm_bindgen::prelude::*;

    use slim_auth::shared_secret::SharedSecret;
    use slim_auth::traits::TokenProvider;
    use slim_datapath::api::{ProtoMessage, ProtoSessionType};
    use slim_datapath::messages::Name;
    use slim_session::interceptor::{IdentityInterceptor, SessionInterceptorProvider};
    use slim_session::notification::Notification;
    use slim_session::runtime::CancellationToken;
    use tokio_with_wasm::sync::mpsc;
    use slim_session::session_controller::SessionController;
    use slim_session::subscription_manager::{SubscriptionManager, SubscriptionOps};
    use slim_session::transmitter::AppTransmitter;
    use slim_session::{Direction, SessionConfig, SessionError, SessionLayer, SlimChannelSender};

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

    /// A SLIM client that connects to a data-plane instance over WebSocket.
    ///
    /// Provides subscribe/unsubscribe, session creation, message publishing,
    /// and event listening for the browser.
    #[wasm_bindgen]
    pub struct SlimClient {
        app_name: Name,
        session_layer: Arc<SessionLayer<SharedSecret, SharedSecret, AppTransmitter>>,
        tx_slim: SlimChannelSender,
        sessions: Arc<parking_lot::Mutex<HashMap<u32, Arc<SessionController>>>>,
        #[allow(clippy::type_complexity)]
        notification_rx: Arc<
            parking_lot::Mutex<
                Option<
                    mpsc::Receiver<
                        Result<Notification, SessionError>,
                    >,
                >,
            >,
        >,
        subscription_manager: SubscriptionManager,
        on_message_cb: Arc<parking_lot::Mutex<Option<JsCallback>>>,
        cancel_token: CancellationToken,
    }

    #[wasm_bindgen]
    impl SlimClient {
        /// Connect to a SLIM data-plane instance via WebSocket.
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
            let app_name = Name::from_strings([org, ns, app]).with_id(0);

            // Auth
            let auth = SharedSecret::new(app, shared_secret)
                .map_err(|e| JsError::new(&format!("auth error: {e}")))?;
            let auth_verifier = SharedSecret::new(app, shared_secret)
                .map_err(|e| JsError::new(&format!("auth error: {e}")))?;
            let token = auth
                .get_token()
                .map_err(|e| JsError::new(&format!("token error: {e}")))?;

            // Build WebSocket URL with auth query parameter
            let mut url = url::Url::parse(endpoint)
                .map_err(|e| JsError::new(&format!("invalid URL: {e}")))?;
            url.query_pairs_mut().append_pair("token", &token);

            let ws = gloo_net::websocket::futures::WebSocket::open(url.as_str())
                .map_err(|e| JsError::new(&format!("websocket error: {e}")))?;

            let (ws_sink, ws_stream) = ws.split();

            // Channels
            // tx_slim: SessionLayer + app methods → outbound to WebSocket
            let (tx_slim, rx_slim) = mpsc::channel(64);
            // tx_app: SessionLayer → app notifications (new sessions)
            let (tx_app, notification_rx) = mpsc::channel(64);

            let cancel_token = CancellationToken::new();

            // Create AppTransmitter (used by SessionLayer for interceptors)
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

            // Create SessionLayer (it creates its own SubscriptionManager internally)
            let conn_id = 0u64;
            let session_layer = Arc::new(SessionLayer::new(
                app_name.clone(),
                auth.clone(),
                auth_verifier,
                conn_id,
                tx_slim.clone(),
                tx_app,
                transmitter,
                Direction::Bidirectional,
                String::new(),
            ));

            // Use the SessionLayer's subscription manager so acks are resolved
            // on the same pending_acks map that session controllers await on.
            let subscription_manager = session_layer.subscription_manager();

            // Self-subscribe (so the gateway knows about us)
            let self_sub = ProtoMessage::builder()
                .source(app_name.clone())
                .destination(app_name.clone())
                .build_subscribe()
                .map_err(|e| JsError::new(&format!("subscribe error: {e}")))?;
            tx_slim
                .send(Ok(self_sub))
                .await
                .map_err(|e| JsError::new(&format!("send error: {e}")))?;

            // ── Spawn WS write loop ──
            // Reads from rx_slim, encodes to protobuf, sends as binary WebSocket frames
            let cancel_write = cancel_token.child_token();
            wasm_bindgen_futures::spawn_local(async move {
                let mut rx = rx_slim;
                let mut sink = ws_sink;
                loop {
                    let msg = tokio_with_wasm::select! {
                        m = rx.recv() => m,
                        _ = cancel_write.cancelled() => None,
                    };
                    match msg {
                        None => break,
                        Some(Ok(proto_msg)) => {
                            let bytes = proto_msg.encode_to_vec();
                            let ws_msg = gloo_net::websocket::Message::Bytes(bytes);
                            if let Err(e) = sink.send(ws_msg).await {
                                tracing::error!("ws send error: {e}");
                                break;
                            }
                        }
                        Some(Err(status)) => {
                            tracing::warn!("outbound error status: {}", status.message());
                        }
                    }
                }
                tracing::info!("ws write loop ended");
            });

            // ── Spawn WS read loop ──
            // Reads binary WebSocket frames, decodes protobuf, routes to SessionLayer
            let sl_clone = session_layer.clone();
            let cancel_read = cancel_token.child_token();
            let sub_mgr_clone = subscription_manager.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let mut stream = ws_stream;
                loop {
                    if cancel_read.is_cancelled() {
                        break;
                    }

                    let next = stream.next().await;
                    match next {
                        None => {
                            tracing::info!("websocket closed by server");
                            break;
                        }
                        Some(Ok(gloo_net::websocket::Message::Bytes(bytes))) => {
                            match ProtoMessage::decode(&bytes[..]) {
                                Ok(msg) => {
                                    route_incoming_message(msg, &sl_clone, &sub_mgr_clone);
                                }
                                Err(e) => {
                                    tracing::warn!("protobuf decode error: {e}");
                                }
                            }
                        }
                        Some(Ok(gloo_net::websocket::Message::Text(_))) => {
                            // Text frames ignored by SLIM protocol
                        }
                        Some(Err(e)) => {
                            tracing::error!("websocket read error: {e}");
                            break;
                        }
                    }
                }
                tracing::info!("ws read loop ended");
            });

            tracing::info!("connected to SLIM at {endpoint} as {app_name}");

            Ok(SlimClient {
                app_name,
                session_layer,
                tx_slim,
                sessions: Arc::new(parking_lot::Mutex::new(HashMap::new())),
                notification_rx: Arc::new(parking_lot::Mutex::new(Some(notification_rx))),
                subscription_manager,
                on_message_cb: Arc::new(parking_lot::Mutex::new(None)),
                cancel_token,
            })
        }

        /// Subscribe to receive messages for the given name (org/ns/name).
        /// Waits for the gateway to acknowledge the subscription.
        #[wasm_bindgen]
        pub async fn subscribe(
            &self,
            org: &str,
            ns: &str,
            name: &str,
        ) -> Result<(), JsError> {
            let sub_name =
                Name::from_strings([org, ns, name]).with_id(self.session_layer.app_id());

            let (subscription_id, ack_rx) = self
                .subscription_manager
                .subscribe(&self.app_name, &sub_name, None)
                .await
                .map_err(|e| JsError::new(&format!("subscribe error: {e}")))?;

            SubscriptionManager::await_ack(ack_rx)
                .await
                .map_err(|e| JsError::new(&format!("subscription rejected: {e}")))?;

            self.session_layer
                .add_app_name(sub_name, subscription_id);
            tracing::info!("subscribed to {org}/{ns}/{name}");
            Ok(())
        }

        /// Unsubscribe from a name.
        #[wasm_bindgen]
        pub async fn unsubscribe(
            &self,
            org: &str,
            ns: &str,
            name: &str,
        ) -> Result<(), JsError> {
            let unsub_name = Name::from_strings([org, ns, name]);

            let subscription_id = self
                .session_layer
                .remove_app_name(&unsub_name)
                .unwrap_or(0);

            let ack_rx = self
                .subscription_manager
                .unsubscribe(&self.app_name, &unsub_name, subscription_id, None)
                .await
                .map_err(|e| JsError::new(&format!("unsubscribe error: {e}")))?;

            SubscriptionManager::await_ack(ack_rx)
                .await
                .map_err(|e| JsError::new(&format!("unsubscription rejected: {e}")))?;

            tracing::info!("unsubscribed from {org}/{ns}/{name}");
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

            self.sessions
                .lock()
                .insert(session_id, session_arc.clone());

            // Spawn a receiver to forward session messages to the JS callback
            let on_msg = self.on_message_cb.clone();
            let (_, mut rx) = session_ctx.into_parts();
            wasm_bindgen_futures::spawn_local(async move {
                while let Some(msg_result) = rx.recv().await {
                    if let Ok(msg) = msg_result {
                        let cb = on_msg.lock().clone();
                        if let Some(callback) = cb {
                            let obj = build_message_js_object(&msg);
                            callback.0.call1(&JsValue::NULL, &obj).ok();
                        }
                    }
                }
            });

            // Wait for session establishment (discovery handshake)
            init_ack
                .await
                .map_err(|e| JsError::new(&format!("session init error: {e}")))?;

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
                                    js_sys::Reflect::set(
                                        &obj,
                                        &"destination".into(),
                                        &dst.into(),
                                    )
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

        /// Disconnect from the SLIM data-plane.
        /// Cancels all background tasks and closes the WebSocket.
        #[wasm_bindgen]
        pub fn disconnect(&self) {
            self.cancel_token.cancel();
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
    }

    /// Route an incoming protobuf message from the WebSocket.
    ///
    /// Subscription acks are resolved inline so the read loop stays responsive.
    /// Publish messages are spawned into a separate task to avoid deadlocking
    /// the read loop when session handling awaits subscription acks.
    fn route_incoming_message(
        msg: ProtoMessage,
        session_layer: &Arc<SessionLayer<SharedSecret, SharedSecret, AppTransmitter>>,
        subscription_manager: &SubscriptionManager,
    ) {
        // Handle subscription acknowledgements inline (must not be spawned)
        if msg.is_subscription_ack() {
            subscription_manager.resolve_ack(msg.get_subscription_ack());
            return;
        }

        // Spawn ALL messages into a separate task to avoid deadlocking the
        // read loop when session handling awaits subscription acks.
        let sl = session_layer.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = sl.handle_message_from_slim(msg).await {
                if !matches!(e, SessionError::SubscriptionNotFound(_)) {
                    tracing::warn!("handle message error: {e}");
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

        obj.into()
    }
}
