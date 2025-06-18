// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::mls::Mls;
use slim_datapath::api::MessageType;
use slim_datapath::api::proto::pubsub::v1::Message;
use slim_service::session::SessionInterceptor;
use std::sync::{Arc, Mutex};

pub struct MlsInterceptor {
    mls: Arc<Mutex<Mls>>,
}

impl MlsInterceptor {
    pub fn new(mls: Arc<Mutex<Mls>>) -> Self {
        Self { mls }
    }
}

impl SessionInterceptor for MlsInterceptor {
    fn on_msg_from_app(&self, msg: &mut Message) {
        let group_id: Vec<u8> = match msg.metadata.get("mls_group_id") {
            Some(id) => {
                use base64::{Engine as _, engine::general_purpose};
                match general_purpose::STANDARD.decode(id) {
                    Ok(bytes) => bytes,
                    Err(_) => return,
                }
            }
            None => return,
        };

        if !msg.is_publish() {
            return;
        }

        let payload = match msg.get_payload() {
            Some(content) => content.blob.clone(),
            None => return,
        };

        let original_payload = payload.clone();

        let encrypted_result = {
            let mut mls_guard = match self.mls.lock() {
                Ok(guard) => guard,
                Err(_) => return,
            };
            if mls_guard.is_group_member(&group_id) {
                mls_guard.encrypt_message(&group_id, &payload)
            } else {
                Ok(payload)
            }
        };

        match encrypted_result {
            Ok(encrypted_payload) => {
                let was_encrypted = encrypted_payload != original_payload;

                if let Some(message_type) = &mut msg.message_type {
                    if let MessageType::Publish(publish) = message_type {
                        if let Some(content) = &mut publish.msg {
                            content.blob = encrypted_payload;
                            if was_encrypted {
                                msg.metadata
                                    .insert("mls_encrypted".to_string(), "true".to_string());
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to encrypt message with MLS: {}", e);
            }
        }
    }

    fn on_msg_from_slim(&self, msg: &mut Message) {
        if !msg.is_publish() {
            return;
        }

        if msg.metadata.get("mls_encrypted").map(|v| v.as_str()) != Some("true") {
            return;
        }

        let payload = match msg.get_payload() {
            Some(content) => content.blob.clone(),
            None => return,
        };

        let decrypted_result = {
            let mut mls_guard = match self.mls.lock() {
                Ok(guard) => guard,
                Err(_) => return,
            };
            if mls_guard.has_any_groups() {
                mls_guard.decrypt_message(&payload)
            } else {
                Ok(payload)
            }
        };

        match decrypted_result {
            Ok(decrypted_payload) => {
                if let Some(message_type) = &mut msg.message_type {
                    if let MessageType::Publish(publish) = message_type {
                        if let Some(content) = &mut publish.msg {
                            content.blob = decrypted_payload;
                            msg.metadata.remove("mls_encrypted");
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to decrypt message with MLS: {}", e);
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
        let interceptor = MlsInterceptor::new(mls_arc);

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
        let interceptor = MlsInterceptor::new(mls_arc);

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
