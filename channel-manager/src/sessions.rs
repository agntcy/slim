// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Thread-safe session list for managing active SLIM channels.

use std::collections::HashMap;
use std::sync::Arc;

use slim_bindings::{App, Session};
use slim_session::SessionError;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// Per-channel state: the SLIM session and its event monitor task.
struct Channel {
    session: Arc<Session>,
    monitor: JoinHandle<()>,
}

/// Thread-safe list of active sessions (channels) managed by the channel manager.
pub struct SessionsList {
    channels: RwLock<HashMap<String, Channel>>,
}

impl Default for SessionsList {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionsList {
    /// Create a new empty sessions list
    pub fn new() -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
        }
    }

    /// Try to insert a session atomically: checks existence and inserts under a single write lock.
    /// Spawns a background event monitor for the session.
    /// Returns `true` if inserted, `false` if the channel already exists.
    pub async fn try_insert_session(
        &self,
        channel_name: String,
        session: Arc<Session>,
    ) -> bool {
        let mut channels = self.channels.write().await;
        if channels.contains_key(&channel_name) {
            return false;
        }
        let monitor = spawn_session_event_monitor(channel_name.clone(), session.clone());
        channels.insert(channel_name, Channel { session, monitor });
        true
    }

    /// Add a session to the list
    pub async fn add_session(
        &self,
        channel_name: String,
        session: Arc<Session>,
    ) -> anyhow::Result<()> {
        let mut channels = self.channels.write().await;
        if channels.contains_key(&channel_name) {
            anyhow::bail!("channel {} already exists", channel_name);
        }
        let monitor = spawn_session_event_monitor(channel_name.clone(), session.clone());
        channels.insert(channel_name, Channel { session, monitor });
        Ok(())
    }

    /// Get a session by channel name
    pub async fn get_session(&self, channel_name: &str) -> Option<Arc<Session>> {
        let channels = self.channels.read().await;
        channels.get(channel_name).map(|c| c.session.clone())
    }

    /// Remove a session by channel name and return it.
    /// Aborts the event monitor task.
    pub async fn remove_session(&self, channel_name: &str) -> Option<Arc<Session>> {
        let mut channels = self.channels.write().await;
        channels.remove(channel_name).map(|channel| {
            channel.monitor.abort();
            channel.session
        })
    }

    /// List all channel names
    pub async fn list_channel_names(&self) -> Vec<String> {
        let channels = self.channels.read().await;
        channels.keys().cloned().collect()
    }

    /// Delete all sessions (used during shutdown)
    pub async fn delete_all(&self, app: &App) {
        let mut channels = self.channels.write().await;
        for (name, channel) in channels.drain() {
            channel.monitor.abort();
            if let Err(e) = app.delete_session_and_wait_async(channel.session).await {
                warn!("Failed to delete session for channel {}: {}", name, e);
            } else {
                info!("Deleted session for channel {}", name);
            }
        }
    }
}

/// Spawn a background task that monitors a session for events.
///
/// The task calls `get_session_message` in a loop, dispatching each event
/// to the appropriate handler. New event types can be added here as the
/// channel manager evolves.
fn spawn_session_event_monitor(channel_name: String, session: Arc<Session>) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match session.get_session_message(None).await {
                Ok(_msg) => {
                    // Application-level data message received on the channel.
                    // The channel manager does not process data messages today,
                    // but this is where future message-level handling would go.
                    debug!(channel = %channel_name, "Received data message (ignored)");
                }
                Err(e) => {
                    if !handle_session_error(&channel_name, &e) {
                        break;
                    }
                }
            }
        }
    })
}

/// Handle a session error from the event monitor.
/// Returns `true` if the monitor should continue, `false` if it should stop.
fn handle_session_error(channel_name: &str, error: &SessionError) -> bool {
    match error {
        SessionError::ParticipantDisconnected(name) => {
            warn!(
                channel = %channel_name,
                participant = %name,
                "Participant disconnected from channel"
            );
            true
        }
        SessionError::ReceiveTimeout => {
            // Timeout — continue listening
            true
        }
        _ => {
            // Fatal session error — stop the monitor
            warn!(
                channel = %channel_name,
                error = %error,
                "Session monitor stopped"
            );
            false
        }
    }
}
