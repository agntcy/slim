// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use uuid::Uuid;

use crate::error::{Error, Result};

use crate::api::proto::controller::proto::v1::{
    Ack, AddParticipantRequest, ControlMessage, DeleteChannelRequest, DeleteParticipantRequest,
    ListChannelsRequest, ListChannelsResponse, ListParticipantsRequest, ListParticipantsResponse,
    control_message::Payload,
};
use crate::api::proto::controlplane::proto::v1::{CreateChannelRequest, CreateChannelResponse};
use crate::db::SharedDb;
use crate::node_control::{DefaultNodeCommandHandler, ResponseKind};

struct Inner {
    db: SharedDb,
    cmd_handler: DefaultNodeCommandHandler,
}

#[derive(Clone)]
pub struct GroupService(Arc<Inner>);

impl GroupService {
    pub fn new(db: SharedDb, cmd_handler: DefaultNodeCommandHandler) -> Self {
        Self(Arc::new(Inner { db, cmd_handler }))
    }

    pub async fn create_channel(
        &self,
        req: CreateChannelRequest,
    ) -> Result<CreateChannelResponse> {
        if req.moderators.is_empty() {
            return Err(Error::InvalidInput("at least one moderator is required to create a channel".to_string()));
        }
        let moderator_name = &req.moderators[0];
        let parts = validate_name(moderator_name, 4)?;
        let rand_id = get_random_id(18)?;
        let channel_name = format!("{}/{}/{}", parts[0], parts[1], rand_id);

        let node_id = self.get_moderator_node(&req.moderators).await?;

        let message_id = Uuid::new_v4().to_string();
        let msg = ControlMessage {
            message_id: message_id.clone(),
            payload: Some(Payload::CreateChannelRequest(
                crate::api::proto::controller::proto::v1::CreateChannelRequest {
                    channel_name: channel_name.clone(),
                    moderators: req.moderators.clone(),
                },
            )),
        };
        self.0.cmd_handler
            .send_message(&node_id, msg)
            .await?;

        let response = self
            .0.cmd_handler
            .wait_for_response(&node_id, ResponseKind::Ack, &message_id)
            .await?;

        let ack = match response.payload {
            Some(Payload::Ack(a)) => a,
            _ => return Err(Error::UnexpectedResponse("expected Ack response".to_string())),
        };
        if !ack.success {
            return Err(Error::InvalidInput(format!("failed to create channel: {:?}", ack.messages)));
        }

        tracing::info!("Channel creation result for {channel_name}: success=true");
        self.0.db.save_channel(&channel_name, req.moderators).await?;
        tracing::info!("Channel saved successfully");

        Ok(CreateChannelResponse { channel_name })
    }

    pub async fn delete_channel(&self, req: DeleteChannelRequest) -> Result<Ack> {
        validate_name(&req.channel_name, 3)?;

        let channel = self
            .0.db
            .get_channel(&req.channel_name)
            .await
            .ok_or_else(|| Error::ChannelNotFound { id: req.channel_name.clone() })?;

        let node_id = self.get_moderator_node(&channel.moderators).await?;

        let mut req = req;
        req.moderators = channel.moderators;

        let message_id = Uuid::new_v4().to_string();
        let msg = ControlMessage {
            message_id: message_id.clone(),
            payload: Some(Payload::DeleteChannelRequest(req.clone())),
        };
        self.0.cmd_handler
            .send_message(&node_id, msg)
            .await?;

        let response = self
            .0.cmd_handler
            .wait_for_response(&node_id, ResponseKind::Ack, &message_id)
            .await?;
        let ack = match response.payload {
            Some(Payload::Ack(a)) => a,
            _ => return Err(Error::UnexpectedResponse("expected Ack response".to_string())),
        };
        if !ack.success {
            return Ok(ack);
        }
        tracing::info!(
            "Channel deletion result for {}: success=true",
            req.channel_name
        );
        self.0.db.delete_channel(&req.channel_name).await?;
        tracing::info!("Channel deleted successfully");
        Ok(ack)
    }

    pub async fn add_participant(&self, req: AddParticipantRequest) -> Result<Ack> {
        validate_name(&req.channel_name, 3)?;
        validate_name(&req.participant_name, 3)?;

        let mut channel = self
            .0.db
            .get_channel(&req.channel_name)
            .await
            .ok_or_else(|| Error::ChannelNotFound { id: req.channel_name.clone() })?;

        let node_id = self.get_moderator_node(&channel.moderators).await?;

        let mut req = req;
        req.moderators = channel.moderators.clone();

        let message_id = Uuid::new_v4().to_string();
        let msg = ControlMessage {
            message_id: message_id.clone(),
            payload: Some(Payload::AddParticipantRequest(req.clone())),
        };
        self.0.cmd_handler
            .send_message(&node_id, msg)
            .await?;

        let response = self
            .0.cmd_handler
            .wait_for_response(&node_id, ResponseKind::Ack, &message_id)
            .await?;
        let ack = match response.payload {
            Some(Payload::Ack(a)) => a,
            _ => return Err(Error::UnexpectedResponse("expected Ack response".to_string())),
        };
        tracing::info!("AddParticipant result success={}", ack.success);
        if !ack.success {
            return Ok(ack);
        }
        if channel.participants.contains(&req.participant_name) {
            return Err(Error::InvalidInput(format!(
                "participant {} already exists in channel {}",
                req.participant_name,
                req.channel_name
            )));
        }
        channel.participants.push(req.participant_name);
        self.0.db.update_channel(channel).await?;
        tracing::info!("Channel updated, participant added successfully.");
        Ok(ack)
    }

    pub async fn delete_participant(&self, req: DeleteParticipantRequest) -> Result<Ack> {
        if req.channel_name.is_empty() {
            return Err(Error::EmptyChannelId);
        }
        if req.participant_name.is_empty() {
            return Err(Error::InvalidInput("participant ID cannot be empty".to_string()));
        }
        let mut channel = self
            .0.db
            .get_channel(&req.channel_name)
            .await
            .ok_or_else(|| Error::ChannelNotFound { id: req.channel_name.clone() })?;

        let idx = channel
            .participants
            .iter()
            .position(|p| p == &req.participant_name)
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "participant {} not found in channel {}",
                    req.participant_name,
                    req.channel_name
                ))
            })?;

        let node_id = self.get_moderator_node(&channel.moderators).await?;

        let mut req = req;
        req.moderators = channel.moderators.clone();

        let message_id = Uuid::new_v4().to_string();
        let msg = ControlMessage {
            message_id: message_id.clone(),
            payload: Some(Payload::DeleteParticipantRequest(req.clone())),
        };
        self.0.cmd_handler
            .send_message(&node_id, msg)
            .await?;

        let response = self
            .0.cmd_handler
            .wait_for_response(&node_id, ResponseKind::Ack, &message_id)
            .await?;
        let ack = match response.payload {
            Some(Payload::Ack(a)) => a,
            _ => return Err(Error::UnexpectedResponse("expected Ack response".to_string())),
        };
        if !ack.success {
            return Ok(ack);
        }
        tracing::info!("DeleteParticipant result success=true");
        channel.participants.remove(idx);
        self.0.db.update_channel(channel).await?;
        tracing::info!("Channel updated, participant deleted successfully");
        Ok(ack)
    }

    pub async fn list_channels(
        &self,
        _req: ListChannelsRequest,
    ) -> Result<ListChannelsResponse> {
        let channels = self.0.db.list_channels().await;
        Ok(ListChannelsResponse {
            original_message_id: String::new(),
            channel_name: channels.into_iter().map(|c| c.id).collect(),
        })
    }

    pub async fn list_participants(
        &self,
        req: ListParticipantsRequest,
    ) -> Result<ListParticipantsResponse> {
        if req.channel_name.is_empty() {
            return Err(Error::EmptyChannelId);
        }
        let channel = self
            .0.db
            .get_channel(&req.channel_name)
            .await
            .ok_or_else(|| Error::ChannelNotFound { id: req.channel_name.clone() })?;
        Ok(ListParticipantsResponse {
            original_message_id: String::new(),
            participant_name: channel.participants,
        })
    }

    pub async fn get_channel_details(
        &self,
        channel_id: &str,
    ) -> Result<crate::db::Channel> {
        self.0.db
            .get_channel(channel_id)
            .await
            .ok_or_else(|| Error::ChannelNotFound { id: channel_id.to_string() })
    }

    async fn get_moderator_node(&self, moderators: &[String]) -> Result<String> {
        if moderators.is_empty() {
            return Err(Error::InvalidInput("no moderators provided".to_string()));
        }
        let moderator = &moderators[0];
        let parts = validate_name(moderator, 4)?;
        let organization = &parts[0];
        let namespace = &parts[1];
        let agent_type = &parts[2];
        // The route table stores component_id as u64-bits reinterpreted as i64.
        let component_id: Option<i64> = parts.get(3).and_then(|s| {
            s.parse::<i64>()
                .ok()
                .or_else(|| s.parse::<u64>().ok().map(|v| v as i64))
        });

        self.0.db
            .get_destination_node_id_for_name(organization, namespace, agent_type, component_id)
            .await
            .ok_or_else(|| Error::InvalidInput(format!("no node found for moderator {moderator}")))
    }
}

/// Splits `name` by `/` and validates it has at least `min_parts` parts.
pub fn validate_name(name: &str, min_parts: usize) -> Result<Vec<String>> {
    let parts: Vec<String> = name.split('/').map(|s| s.to_string()).collect();
    if parts.len() < min_parts {
        return Err(Error::InvalidInput(format!(
            "name '{name}' must have at least {min_parts} slash-separated parts"
        )));
    }
    Ok(parts)
}

/// Returns a random alphanumeric string of `len` characters.
pub fn get_random_id(len: usize) -> Result<String> {
    let id = Uuid::new_v4().to_string().replace('-', "");
    Ok(id[..len.min(id.len())].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{inmemory::InMemoryDb, model::Channel};
    use crate::node_control::DefaultNodeCommandHandler;

    fn make_db() -> crate::db::SharedDb {
        InMemoryDb::shared()
    }

    fn make_service(db: crate::db::SharedDb) -> GroupService {
        let handler = DefaultNodeCommandHandler::new();
        GroupService::new(db, handler)
    }

    // ── validate_name ──────────────────────────────────────────────────────

    #[test]
    fn validate_name_ok() {
        let parts = validate_name("a/b/c/d", 4).unwrap();
        assert_eq!(parts, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn validate_name_too_few_parts() {
        assert!(validate_name("a/b", 4).is_err());
    }

    #[test]
    fn validate_name_exact_min() {
        assert!(validate_name("a/b/c", 3).is_ok());
    }

    #[test]
    fn validate_name_more_than_min() {
        assert!(validate_name("a/b/c/d/e", 3).is_ok());
    }

    // ── get_random_id ──────────────────────────────────────────────────────

    #[test]
    fn get_random_id_length() {
        let id = get_random_id(18).unwrap();
        assert_eq!(id.len(), 18);
    }

    #[test]
    fn get_random_id_zero_length() {
        let id = get_random_id(0).unwrap();
        assert_eq!(id.len(), 0);
    }

    #[test]
    fn get_random_id_capped_at_uuid_hex_len() {
        // UUID v4 without dashes is 32 chars; requesting 100 should give 32.
        let id = get_random_id(100).unwrap();
        assert_eq!(id.len(), 32);
    }

    #[test]
    fn get_random_id_no_dashes() {
        let id = get_random_id(32).unwrap();
        assert!(!id.contains('-'));
    }

    // ── list_channels ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_channels_empty() {
        let db = make_db();
        let svc = make_service(db);
        let resp = svc.list_channels(ListChannelsRequest {}).await.unwrap();
        assert!(resp.channel_name.is_empty());
    }

    #[tokio::test]
    async fn list_channels_returns_all() {
        let db = make_db();
        db.save_channel("c1", vec![]).await.unwrap();
        db.save_channel("c2", vec![]).await.unwrap();
        let svc = make_service(db);
        let resp = svc.list_channels(ListChannelsRequest {}).await.unwrap();
        assert_eq!(resp.channel_name.len(), 2);
    }

    // ── list_participants ──────────────────────────────────────────────────

    #[tokio::test]
    async fn list_participants_ok() {
        let db = make_db();
        db.save_channel("chan", vec!["mod".to_string()])
            .await
            .unwrap();
        db.update_channel(Channel {
            id: "chan".to_string(),
            moderators: vec!["mod".to_string()],
            participants: vec!["p1".to_string(), "p2".to_string()],
        })
        .await
        .unwrap();
        let svc = make_service(db);
        let resp = svc
            .list_participants(ListParticipantsRequest {
                channel_name: "chan".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(resp.participant_name.len(), 2);
    }

    #[tokio::test]
    async fn list_participants_empty_channel_id_returns_error() {
        let db = make_db();
        let svc = make_service(db);
        assert!(
            svc.list_participants(ListParticipantsRequest {
                channel_name: String::new(),
            })
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn list_participants_not_found_returns_error() {
        let db = make_db();
        let svc = make_service(db);
        assert!(
            svc.list_participants(ListParticipantsRequest {
                channel_name: "ghost".to_string(),
            })
            .await
            .is_err()
        );
    }

    // ── get_channel_details ────────────────────────────────────────────────

    #[tokio::test]
    async fn get_channel_details_ok() {
        let db = make_db();
        db.save_channel("chan", vec!["mod".to_string()])
            .await
            .unwrap();
        let svc = make_service(db);
        let ch = svc.get_channel_details("chan").await.unwrap();
        assert_eq!(ch.id, "chan");
    }

    #[tokio::test]
    async fn get_channel_details_not_found() {
        let db = make_db();
        let svc = make_service(db);
        assert!(svc.get_channel_details("ghost").await.is_err());
    }

    // ── create_channel validation ──────────────────────────────────────────

    #[tokio::test]
    async fn create_channel_no_moderators_returns_error() {
        let db = make_db();
        let svc = make_service(db);
        let result = svc
            .create_channel(CreateChannelRequest { moderators: vec![] })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("moderator"));
    }

    #[tokio::test]
    async fn create_channel_moderator_too_few_parts_returns_error() {
        let db = make_db();
        let svc = make_service(db);
        // Moderator name must have ≥ 4 slash-separated parts.
        let result = svc
            .create_channel(CreateChannelRequest {
                moderators: vec!["a/b/c".to_string()],
            })
            .await;
        assert!(result.is_err());
    }

    // ── delete_channel validation ──────────────────────────────────────────

    #[tokio::test]
    async fn delete_channel_bad_name_returns_error() {
        let db = make_db();
        let svc = make_service(db);
        let result = svc
            .delete_channel(DeleteChannelRequest {
                channel_name: "bad".to_string(),
                moderators: vec![],
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn delete_channel_not_found_returns_error() {
        let db = make_db();
        let svc = make_service(db);
        let result = svc
            .delete_channel(DeleteChannelRequest {
                channel_name: "a/b/c".to_string(),
                moderators: vec![],
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }
}
