// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::{errors::SessionError, session::SessionInterceptor};
use parking_lot::Mutex;
use slim_datapath::api::MessageType;
use slim_datapath::api::proto::pubsub::v1::Message;
use slim_mls::mls::Mls;
use std::sync::Arc;
use tracing::{debug, error, warn};

// Metadata Keys
pub const METADATA_MLS_ENABLED: &str = "MLS_ENABLED";
pub const METADATA_MLS_INIT_COMMIT_ID: &str = "MLS_INIT_COMMIT_ID";
const METADATA_MLS_ENCRYPTED: &str = "MLS_ENCRYPTED";
const METADATA_MLS_GROUP_ID: &str = "MLS_GROUP_ID";

pub struct MlsInterceptor {
    mls: Arc<Mutex<Mls>>,
}

impl MlsInterceptor {
    pub fn new(mls: Arc<Mutex<Mls>>) -> Self {
        Self { mls }
    }
}

impl SessionInterceptor for MlsInterceptor {
    fn on_msg_from_app(&self, msg: &mut Message) -> Result<(), SessionError> {
        // Only process Publish message types
        if !msg.is_publish() {
            debug!("Skipping non-Publish message type in encryption path");
            return Ok(());
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
        let (encrypted_payload, group_id) = match &binding {
            Ok(res) => res,
            Err(e) => {
                error!(
                    "Failed to encrypt message with MLS: {}, dropping message",
                    e
                );
                return Err(SessionError::InterceptorError(format!(
                    "MLS encryption failed: {}",
                    e
                )));
            }
        };

        if let Some(MessageType::Publish(publish)) = &mut msg.message_type {
            if let Some(content) = &mut publish.msg {
                content.blob = encrypted_payload.to_vec();
                msg.insert_metadata(METADATA_MLS_ENCRYPTED.to_owned(), "true".to_owned());
                msg.insert_metadata(METADATA_MLS_GROUP_ID.to_owned(), group_id.to_string());
            }
        }
        Ok(())
    }

    fn on_msg_from_slim(&self, msg: &mut Message) -> Result<(), SessionError> {
        // Only process Publish message types
        if !msg.is_publish() {
            debug!("Skipping non-Publish message type in decryption path");
            return Ok(());
        }

        let is_encrypted =
            msg.metadata.get(METADATA_MLS_ENCRYPTED).map(|v| v.as_str()) == Some("true");

        if !is_encrypted {
            debug!("Message not marked as encrypted, skipping decryption");
            return Ok(());
        }

        let group_id = match msg.metadata.get(METADATA_MLS_GROUP_ID) {
            Some(id) => id,
            None => {
                warn!("Message missing MLS_GROUP_ID metadata");
                return Err(SessionError::InterceptorError(
                    "Message missing MLS_GROUP_ID metadata".to_string(),
                ));
            }
        };

        let payload = match msg.get_payload() {
            Some(content) => &content.blob,
            None => {
                warn!("Encrypted message has no payload");
                return Err(SessionError::InterceptorError(
                    "Encrypted message has no payload".to_string(),
                ));
            }
        };

        let decrypted_payload = {
            let mut mls_guard = self.mls.lock();

            debug!("Decrypting message for group member");
            match mls_guard.decrypt_message(group_id, payload) {
                Ok(decrypted_payload) => decrypted_payload,
                Err(e) => {
                    error!("Failed to decrypt message with MLS: {}", e);
                    return Err(SessionError::InterceptorError(format!(
                        "MLS decryption failed: {}",
                        e
                    )));
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
    use slim_mls::identity::FileBasedIdentityProvider;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_mls_interceptor_without_group() {
        let identity_provider =
            Arc::new(FileBasedIdentityProvider::new("/tmp/test_mls_interceptor_no_group").unwrap());
        let mut mls = Mls::new("test_user".to_string(), identity_provider);
        mls.initialize().await.unwrap();

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
        msg.insert_metadata(METADATA_MLS_GROUP_ID.to_string(), "test_group".to_string());

        let result = interceptor.on_msg_from_app(&mut msg);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("MLS group does not exists")
        );
    }

    #[tokio::test]
    async fn test_mls_interceptor_with_group() {
        let alice_provider =
            Arc::new(FileBasedIdentityProvider::new("/tmp/test_mls_interceptor_alice").unwrap());
        let bob_provider =
            Arc::new(FileBasedIdentityProvider::new("/tmp/test_mls_interceptor_bob").unwrap());

        let mut alice_mls = Mls::new("alice".to_string(), alice_provider);
        let mut bob_mls = Mls::new("bob".to_string(), bob_provider);

        alice_mls.initialize().await.unwrap();
        bob_mls.initialize().await.unwrap();

        let group_id = alice_mls.create_group().unwrap();
        let bob_key_package = bob_mls.generate_key_package().unwrap();
        let (_, welcome_message) = alice_mls.add_member(&bob_key_package).unwrap();
        bob_mls.process_welcome(&welcome_message).unwrap();

        let alice_interceptor = MlsInterceptor::new(Arc::new(Mutex::new(alice_mls)));
        let bob_interceptor = MlsInterceptor::new(Arc::new(Mutex::new(bob_mls)));

        use base64::{Engine as _, engine::general_purpose};
        let group_id_str = general_purpose::STANDARD.encode(&group_id);

        let original_payload = b"Hello from Alice!";
        let mut alice_msg = Message::new_publish(
            &slim_datapath::messages::Agent::from_strings("org", "default", "alice", 0),
            &slim_datapath::messages::AgentType::from_strings("org", "default", "bob"),
            None,
            None,
            "text",
            original_payload.to_vec(),
        );
        alice_msg.insert_metadata(METADATA_MLS_GROUP_ID.to_string(), group_id_str);

        alice_interceptor.on_msg_from_app(&mut alice_msg).unwrap();

        assert_ne!(alice_msg.get_payload().unwrap().blob, original_payload);
        assert_eq!(
            alice_msg
                .metadata
                .get(METADATA_MLS_ENCRYPTED)
                .map(|v| v.as_str()),
            Some("true")
        );

        let mut bob_msg = alice_msg.clone();
        bob_interceptor.on_msg_from_slim(&mut bob_msg).unwrap();

        assert_eq!(bob_msg.get_payload().unwrap().blob, original_payload);
        assert_eq!(bob_msg.metadata.get(METADATA_MLS_ENCRYPTED), None);
    }

    #[tokio::test]
    async fn test_mls_interceptor_non_encrypted_message() {
        let identity_provider = Arc::new(
            FileBasedIdentityProvider::new("/tmp/test_mls_interceptor_non_encrypted").unwrap(),
        );
        let mut mls = Mls::new("test_user".to_string(), identity_provider);
        mls.initialize().await.unwrap();
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

        interceptor.on_msg_from_slim(&mut msg).unwrap();
        assert_eq!(msg.get_payload().unwrap().blob, b"plain text message");
    }
}
