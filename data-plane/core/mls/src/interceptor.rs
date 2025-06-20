// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::mls::Mls;
use parking_lot::Mutex;
use slim_datapath::api::MessageType;
use slim_datapath::api::proto::pubsub::v1::Message;
use slim_service::session::SessionInterceptor;
use std::sync::Arc;
use tracing::warn;

pub struct MlsInterceptor {
    mls: Arc<Mutex<Mls>>,
    group_id: Vec<u8>,
}

impl MlsInterceptor {
    pub fn new(mls: Arc<Mutex<Mls>>, group_id: Vec<u8>) -> Self {
        Self { mls, group_id }
    }
}

impl SessionInterceptor for MlsInterceptor {
    fn on_msg_from_app(&self, msg: &mut Message) {
        warn!("MLS interceptor on_msg_from_app called");

        if !msg.is_publish() {
            warn!("Message is not publish type, skipping MLS");
            return;
        }

        use base64::{Engine as _, engine::general_purpose};
        let group_id_b64 = general_purpose::STANDARD.encode(&self.group_id);
        warn!("Added group_id metadata: {}", group_id_b64);
        msg.metadata
            .insert("mls_group_id".to_string(), group_id_b64);

        let payload = match msg.get_payload() {
            Some(content) => content.blob.clone(),
            None => return,
        };

        let original_payload = payload.clone();

        let encrypted_result = {
            let mut mls_guard = self.mls.lock();
            let is_member = mls_guard.is_group_member(&self.group_id);
            warn!(
                "Is group member: {}, group_id len: {}",
                is_member,
                self.group_id.len()
            );

            if is_member {
                warn!("Attempting to encrypt message");
                mls_guard.encrypt_message(&self.group_id, &payload)
            } else {
                warn!("Not a group member, skipping encryption");
                Ok(payload)
            }
        };

        match encrypted_result {
            Ok(encrypted_payload) => {
                let was_encrypted = encrypted_payload != original_payload;

                if was_encrypted {
                    warn!("MLS ENCRYPTION:");
                    warn!(
                        "   Original: {:?}",
                        String::from_utf8_lossy(&original_payload)
                    );
                    warn!("   Encrypted length: {} bytes", encrypted_payload.len());
                }

                if let Some(MessageType::Publish(publish)) = &mut msg.message_type {
                    if let Some(content) = &mut publish.msg {
                        content.blob = encrypted_payload;
                        if was_encrypted {
                            msg.metadata
                                .insert("mls_encrypted".to_string(), "true".to_string());
                        }
                    }
                }
            }
            Err(e) => {
                warn!("Failed to encrypt message with MLS: {}", e);
            }
        }
    }

    fn on_msg_from_slim(&self, msg: &mut Message) {
        warn!("MLS interceptor on_msg_from_slim called");

        if !msg.is_publish() {
            warn!("Message is not publish type in on_msg_from_slim");
            return;
        }

        let is_encrypted = msg.metadata.get("mls_encrypted").map(|v| v.as_str()) == Some("true");
        warn!(
            "Message mls_encrypted metadata: {:?}",
            msg.metadata.get("mls_encrypted")
        );

        if !is_encrypted {
            warn!("Message not marked as MLS encrypted, skipping decryption");
            return;
        }

        let payload = match msg.get_payload() {
            Some(content) => content.blob.clone(),
            None => return,
        };

        warn!("MLS DECRYPTION:");
        warn!("   Encrypted length: {} bytes", payload.len());

        let decrypted_result = {
            let mut mls_guard = self.mls.lock();
            if mls_guard.has_any_groups() {
                mls_guard.decrypt_message(&payload)
            } else {
                Ok(payload)
            }
        };

        match decrypted_result {
            Ok(decrypted_payload) => {
                warn!(
                    "   Decrypted: {:?}",
                    String::from_utf8_lossy(&decrypted_payload)
                );

                if let Some(MessageType::Publish(publish)) = &mut msg.message_type {
                    if let Some(content) = &mut publish.msg {
                        content.blob = decrypted_payload;
                        msg.metadata.remove("mls_encrypted");
                    }
                }
            }
            Err(e) => {
                warn!("Failed to decrypt message with MLS: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::FileBasedIdentityProvider;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_mls_interceptor_without_group() {
        let identity_provider =
            Arc::new(FileBasedIdentityProvider::new("/tmp/test_mls_interceptor_no_group").unwrap());
        let mut mls = Mls::new("test_user".to_string(), identity_provider);
        mls.initialize().await.unwrap();

        let mls_arc = Arc::new(Mutex::new(mls));
        let interceptor = MlsInterceptor::new(mls_arc, vec![1, 2, 3]);

        let mut msg = Message::new_publish(
            &slim_datapath::messages::Agent::from_strings("org", "default", "test", 0),
            &slim_datapath::messages::AgentType::from_strings("org", "default", "target"),
            None,
            None,
            "text",
            b"test message".to_vec(),
        );
        msg.metadata
            .insert("mls_group_id".to_string(), "test_group".to_string());

        interceptor.on_msg_from_app(&mut msg);
        assert_eq!(msg.get_payload().unwrap().blob, b"test message");
        assert_eq!(msg.metadata.get("mls_encrypted"), None);
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
        let welcome_message = alice_mls.add_member(&group_id, &bob_key_package).unwrap();
        bob_mls.join_group(&welcome_message).unwrap();

        let alice_interceptor =
            MlsInterceptor::new(Arc::new(Mutex::new(alice_mls)), group_id.clone());
        let bob_interceptor = MlsInterceptor::new(Arc::new(Mutex::new(bob_mls)), group_id.clone());

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
        alice_msg
            .metadata
            .insert("mls_group_id".to_string(), group_id_str);

        alice_interceptor.on_msg_from_app(&mut alice_msg);

        assert_ne!(alice_msg.get_payload().unwrap().blob, original_payload);
        assert_eq!(
            alice_msg.metadata.get("mls_encrypted").map(|v| v.as_str()),
            Some("true")
        );

        let mut bob_msg = alice_msg.clone();
        bob_interceptor.on_msg_from_slim(&mut bob_msg);

        assert_eq!(bob_msg.get_payload().unwrap().blob, original_payload);
        assert_eq!(bob_msg.metadata.get("mls_encrypted"), None);
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
        let interceptor = MlsInterceptor::new(mls_arc, vec![1, 2, 3]);

        let mut msg = Message::new_publish(
            &slim_datapath::messages::Agent::from_strings("org", "default", "sender", 0),
            &slim_datapath::messages::AgentType::from_strings("org", "default", "receiver"),
            None,
            None,
            "text",
            b"plain text message".to_vec(),
        );

        interceptor.on_msg_from_slim(&mut msg);
        assert_eq!(msg.get_payload().unwrap().blob, b"plain text message");
    }
}
