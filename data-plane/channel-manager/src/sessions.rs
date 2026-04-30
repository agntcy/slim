// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Thread-safe session list for managing active SLIM channels.

use std::collections::HashMap;
use std::sync::Arc;

use slim_bindings::{App, Session};
use slim_session::SessionError;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

/// Per-channel state: the SLIM session and its event monitor task.
struct Channel {
    session: Arc<Session>,
    monitor: JoinHandle<()>,
}

/// Thread-safe list of active sessions (channels) managed by the channel manager.
#[derive(Default)]
pub struct SessionsList {
    channels: RwLock<HashMap<String, Channel>>,
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
    pub async fn try_insert_session(&self, channel_name: String, session: Arc<Session>) -> bool {
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

    /// Remove and delete a session by channel name: aborts the monitor,
    /// deletes the SLIM session, and removes it from the map.
    pub async fn remove_session(&self, channel_name: &str, app: &App) -> anyhow::Result<()> {
        // Remove from map and release the lock before the potentially slow SLIM call
        let session = {
            let mut channels = self.channels.write().await;
            let channel = channels
                .remove(channel_name)
                .ok_or_else(|| anyhow::anyhow!("channel {channel_name} not found"))?;
            channel.monitor.abort();
            channel.session
        };

        // Delete the SLIM session with a timeout to avoid hanging
        let completion = app
            .delete_session_async(session)
            .await
            .map_err(|e| anyhow::anyhow!("failed to delete session for {channel_name}: {e}"))?;

        match completion
            .wait_for_async(std::time::Duration::from_secs(5))
            .await
        {
            Ok(()) => Ok(()),
            Err(e) => {
                warn!(
                    channel = %channel_name,
                    "Session deletion did not complete in time: {e}"
                );
                Ok(()) // Session was already removed from the map
            }
        }
    }

    /// List all channel names
    pub async fn list_channel_names(&self) -> Vec<String> {
        let channels = self.channels.read().await;
        channels.keys().cloned().collect()
    }

    /// Delete all sessions (used during shutdown)
    pub async fn delete_all(&self, app: &App) {
        let names: Vec<String> = self.list_channel_names().await;
        for name in names {
            match self.remove_session(&name, app).await {
                Ok(()) => info!("Deleted session for channel {name}"),
                Err(e) => warn!("{e}"),
            }
        }
    }
}

/// Spawn a background task that monitors a session for events.
///
/// The task calls `get_session_message` in a loop, dispatching each event
/// to the appropriate handler.
fn spawn_session_event_monitor(channel_name: String, session: Arc<Session>) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match session.get_session_message(None).await {
                Ok(_msg) => {
                    // Application-level data message received on the channel.
                    // The channel manager does not process data messages.
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
            info!(
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
            error!(
                channel = %channel_name,
                error = %error,
                "Session monitor stopped"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_datapath::messages::Name as DatapathName;

    // ── handle_session_error ─────────────────────────────────────────────

    #[test]
    fn test_participant_disconnected_continues() {
        let name = DatapathName::from_strings(["org", "ns", "participant"]);
        let err = SessionError::ParticipantDisconnected(name);
        assert!(handle_session_error("test/channel/1", &err));
    }

    #[test]
    fn test_receive_timeout_continues() {
        assert!(handle_session_error(
            "test/channel/1",
            &SessionError::ReceiveTimeout
        ));
    }

    // ── Helper: create a dummy Session for testing ─────────────────────

    /// Build an `Arc<Session>` backed by a dead `Weak` and a real channel.
    fn dummy_session() -> Arc<Session> {
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Arc::new(Session {
            session: std::sync::Weak::new(),
            rx: tokio::sync::RwLock::new(rx),
        })
    }

    #[tokio::test]
    async fn test_try_insert_session_success() {
        let sessions = SessionsList::new();
        let s = dummy_session();
        assert!(sessions.try_insert_session("a/b/c".into(), s).await);
        assert!(sessions.get_session("a/b/c").await.is_some());
    }

    #[tokio::test]
    async fn test_try_insert_session_duplicate_returns_false() {
        let sessions = SessionsList::new();
        assert!(
            sessions
                .try_insert_session("a/b/c".into(), dummy_session())
                .await
        );
        assert!(
            !sessions
                .try_insert_session("a/b/c".into(), dummy_session())
                .await
        );
        // Only one channel should exist
        assert_eq!(sessions.list_channel_names().await.len(), 1);
    }

    #[tokio::test]
    async fn test_add_session_success() {
        let sessions = SessionsList::new();
        assert!(
            sessions
                .add_session("a/b/c".into(), dummy_session())
                .await
                .is_ok()
        );
        assert!(sessions.get_session("a/b/c").await.is_some());
    }

    #[tokio::test]
    async fn test_add_session_duplicate_returns_error() {
        let sessions = SessionsList::new();
        sessions
            .add_session("a/b/c".into(), dummy_session())
            .await
            .unwrap();
        let err = sessions.add_session("a/b/c".into(), dummy_session()).await;
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("already exists"));
    }

    #[tokio::test]
    async fn test_insert_multiple_channels() {
        let sessions = SessionsList::new();
        sessions
            .try_insert_session("ch/1/a".into(), dummy_session())
            .await;
        sessions
            .try_insert_session("ch/2/b".into(), dummy_session())
            .await;
        sessions
            .try_insert_session("ch/3/c".into(), dummy_session())
            .await;
        assert_eq!(sessions.list_channel_names().await.len(), 3);
    }

    #[tokio::test]
    async fn test_new_sessions_list_is_empty() {
        let sessions = SessionsList::new();
        assert!(sessions.list_channel_names().await.is_empty());
    }

    #[tokio::test]
    async fn test_default_sessions_list_is_empty() {
        let sessions = SessionsList::default();
        assert!(sessions.list_channel_names().await.is_empty());
    }

    #[tokio::test]
    async fn test_get_nonexistent_session() {
        let sessions = SessionsList::new();
        assert!(sessions.get_session("nonexistent").await.is_none());
    }
}
