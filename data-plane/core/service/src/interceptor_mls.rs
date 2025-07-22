// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::{errors::SessionError, interceptor::SessionInterceptor};
use parking_lot::Mutex;
use slim_datapath::api::ProtoMessage as Message;
use slim_datapath::api::{MessageType, ProtoSessionMessageType};
use slim_mls::mls::Mls;
use std::sync::Arc;
use tracing::{debug, error, warn};

// Metadata Keys
pub const METADATA_MLS_ENABLED: &str = "MLS_ENABLED";
pub const METADATA_MLS_INIT_COMMIT_ID: &str = "MLS_INIT_COMMIT_ID";
const METADATA_MLS_ENCRYPTED: &str = "MLS_ENCRYPTED";

pub struct MlsInterceptor<P, V>
where
    P: slim_auth::traits::TokenProvider + Send + Sync + Clone + 'static,
    V: slim_auth::traits::Verifier + Send + Sync + Clone + 'static,
{
    mls: Arc<Mutex<Mls<P, V>>>,
}

impl<P, V> MlsInterceptor<P, V>
where
    P: slim_auth::traits::TokenProvider + Send + Sync + Clone + 'static,
    V: slim_auth::traits::Verifier + Send + Sync + Clone + 'static,
{
    pub fn new(mls: Arc<Mutex<Mls<P, V>>>) -> Self {
        Self { mls }
    }
}

#[async_trait::async_trait]
impl<P, V> SessionInterceptor for MlsInterceptor<P, V>
where
    P: slim_auth::traits::TokenProvider + Send + Sync + Clone + 'static,
    V: slim_auth::traits::Verifier + Send + Sync + Clone + 'static,
{
    async fn on_msg_from_app(&self, msg: &mut Message) -> Result<(), SessionError> {
        // Only process Publish message types
        if !msg.is_publish() {
            debug!("Skipping non-Publish message type in encryption path");
            return Ok(());
        }

        match msg.get_session_header().session_message_type() {
            ProtoSessionMessageType::ChannelDiscoveryRequest
            | ProtoSessionMessageType::ChannelDiscoveryReply
            | ProtoSessionMessageType::ChannelJoinRequest
            | ProtoSessionMessageType::ChannelJoinReply
            | ProtoSessionMessageType::ChannelLeaveRequest
            | ProtoSessionMessageType::ChannelLeaveReply
            | ProtoSessionMessageType::ChannelMlsCommit
            | ProtoSessionMessageType::ChannelMlsWelcome
            | ProtoSessionMessageType::ChannelMlsProposal
            | ProtoSessionMessageType::ChannelMlsAck => {
                debug!("Skipping channel messages type in encryption path");
                return Ok(());
            }
            _ => {}
        }

        let payload = match msg.get_payload() {
            Some(content) => &content.blob,
            None => {
                warn!("Message has no payload, skipping MLS processing");
                return Ok(());
            }
        };

        let mut mls_guard = self.mls.lock();

        debug!("Encrypting message for group member");
        let binding = mls_guard.encrypt_message(payload);
        let encrypted_payload = match &binding {
            Ok(res) => res,
            Err(e) => {
                error!(
                    "Failed to encrypt message with MLS: {}, dropping message",
                    e
                );
                return Err(SessionError::MlsEncryptionFailed(e.to_string()));
            }
        };

        if let Some(MessageType::Publish(publish)) = &mut msg.message_type {
            if let Some(content) = &mut publish.msg {
                content.blob = encrypted_payload.to_vec();
                msg.insert_metadata(METADATA_MLS_ENCRYPTED.to_owned(), "true".to_owned());
            }
        }
        Ok(())
    }

    async fn on_msg_from_slim(&self, msg: &mut Message) -> Result<(), SessionError> {
        // Only process Publish message types
        if !msg.is_publish() {
            debug!("Skipping non-Publish message type in decryption path");
            return Ok(());
        }

        match msg.get_session_header().session_message_type() {
            ProtoSessionMessageType::ChannelDiscoveryRequest
            | ProtoSessionMessageType::ChannelDiscoveryReply
            | ProtoSessionMessageType::ChannelJoinRequest
            | ProtoSessionMessageType::ChannelJoinReply
            | ProtoSessionMessageType::ChannelLeaveRequest
            | ProtoSessionMessageType::ChannelLeaveReply
            | ProtoSessionMessageType::ChannelMlsCommit
            | ProtoSessionMessageType::ChannelMlsWelcome
            | ProtoSessionMessageType::ChannelMlsProposal
            | ProtoSessionMessageType::ChannelMlsAck => {
                debug!("Skipping channel messages type in decryption path");
                return Ok(());
            }
            _ => {}
        }

        let is_encrypted =
            msg.metadata.get(METADATA_MLS_ENCRYPTED).map(|v| v.as_str()) == Some("true");

        if !is_encrypted {
            debug!("Message not marked as encrypted, skipping decryption");
            return Ok(());
        }

        let payload = match msg.get_payload() {
            Some(content) => &content.blob,
            None => {
                warn!("Encrypted message has no payload");
                return Err(SessionError::MlsNoPayload);
            }
        };

        let decrypted_payload = {
            let mut mls_guard = self.mls.lock();

            debug!("Decrypting message for group member");
            match mls_guard.decrypt_message(payload) {
                Ok(decrypted_payload) => decrypted_payload,
                Err(e) => {
                    error!("Failed to decrypt message with MLS: {}", e);
                    return Err(SessionError::MlsDecryptionFailed(e.to_string()));
                }
            }
        };

        if let Some(MessageType::Publish(publish)) = &mut msg.message_type {
            if let Some(content) = &mut publish.msg {
                content.blob = decrypted_payload;
                msg.remove_metadata(METADATA_MLS_ENCRYPTED);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_auth::shared_secret::SharedSecret;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_mls_interceptor_without_group() {
        let agent = slim_datapath::messages::Agent::from_strings("org", "default", "test_user", 0);
        let mut mls = Mls::new(
            agent,
            SharedSecret::new("test", "group"),
            SharedSecret::new("test", "group"),
            std::path::PathBuf::from("/tmp/mls_interceptor_test_without_group"),
        );
        mls.initialize().unwrap();

        let mls_arc = Arc::new(Mutex::new(mls));
        let interceptor = MlsInterceptor::new(mls_arc);

        let mut msg = Message::new_publish(
            &slim_datapath::messages::Agent::from_strings("org", "default", "test", 0),
            &slim_datapath::messages::AgentType::from_strings("org", "default", "target"),
            None,
            None,
            "text",
            b"test message".to_vec(),
        );

        let result = interceptor.on_msg_from_app(&mut msg).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("MLS group does not exist")
        );
    }

    #[tokio::test]
    async fn test_mls_interceptor_with_group() {
        let alice_agent =
            slim_datapath::messages::Agent::from_strings("org", "default", "alice", 0);
        let bob_agent = slim_datapath::messages::Agent::from_strings("org", "default", "bob", 1);

        let mut alice_mls = Mls::new(
            alice_agent,
            SharedSecret::new("alice", "group"),
            SharedSecret::new("alice", "group"),
            std::path::PathBuf::from("/tmp/mls_interceptor_test_alice"),
        );
        let mut bob_mls = Mls::new(
            bob_agent,
            SharedSecret::new("bob", "group"),
            SharedSecret::new("bob", "group"),
            std::path::PathBuf::from("/tmp/mls_interceptor_test_bob"),
        );

        alice_mls.initialize().unwrap();
        bob_mls.initialize().unwrap();

        let _group_id = alice_mls.create_group().unwrap();
        let bob_key_package = bob_mls.generate_key_package().unwrap();
        let ret = alice_mls.add_member(&bob_key_package).unwrap();
        bob_mls.process_welcome(&ret.welcome_message).unwrap();

        let alice_interceptor = MlsInterceptor::new(Arc::new(Mutex::new(alice_mls)));
        let bob_interceptor = MlsInterceptor::new(Arc::new(Mutex::new(bob_mls)));

        let original_payload = b"Hello from Alice!";
        let mut alice_msg = Message::new_publish(
            &slim_datapath::messages::Agent::from_strings("org", "default", "alice", 0),
            &slim_datapath::messages::AgentType::from_strings("org", "default", "bob"),
            None,
            None,
            "text",
            original_payload.to_vec(),
        );

        alice_interceptor
            .on_msg_from_app(&mut alice_msg)
            .await
            .unwrap();

        assert_ne!(alice_msg.get_payload().unwrap().blob, original_payload);
        assert_eq!(
            alice_msg
                .metadata
                .get(METADATA_MLS_ENCRYPTED)
                .map(|v| v.as_str()),
            Some("true")
        );

        let mut bob_msg = alice_msg.clone();
        bob_interceptor
            .on_msg_from_slim(&mut bob_msg)
            .await
            .unwrap();

        assert_eq!(bob_msg.get_payload().unwrap().blob, original_payload);
        assert_eq!(bob_msg.metadata.get(METADATA_MLS_ENCRYPTED), None);
    }

    #[tokio::test]
    async fn test_mls_interceptor_non_encrypted_message() {
        let agent = slim_datapath::messages::Agent::from_strings("org", "default", "test_user", 0);
        let mut mls = Mls::new(
            agent,
            SharedSecret::new("test", "group"),
            SharedSecret::new("test", "group"),
            std::path::PathBuf::from("/tmp/mls_interceptor_test_non_encrypted"),
        );
        mls.initialize().unwrap();
        mls.create_group().unwrap();

        let mls_arc = Arc::new(Mutex::new(mls));
        let interceptor = MlsInterceptor::new(mls_arc);

        let mut msg = Message::new_publish(
            &slim_datapath::messages::Agent::from_strings("org", "default", "sender", 0),
            &slim_datapath::messages::AgentType::from_strings("org", "default", "receiver"),
            None,
            None,
            "text",
            b"plain text message".to_vec(),
        );

        interceptor.on_msg_from_slim(&mut msg).await.unwrap();
        assert_eq!(msg.get_payload().unwrap().blob, b"plain text message");
    }
}
