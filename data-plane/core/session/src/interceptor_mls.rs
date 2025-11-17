// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::sync::Arc;

// Third-party crates
use tokio::sync::Mutex;
use tracing::{debug, error};

use slim_datapath::api::ProtoSessionMessageType;
use slim_datapath::api::{ApplicationPayload, ProtoMessage as Message};
use slim_mls::mls::Mls;

// Local crate
use crate::{errors::SessionError, interceptor::SessionInterceptor};

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
            ProtoSessionMessageType::DiscoveryRequest
            | ProtoSessionMessageType::DiscoveryReply
            | ProtoSessionMessageType::JoinRequest
            | ProtoSessionMessageType::JoinReply
            | ProtoSessionMessageType::LeaveRequest
            | ProtoSessionMessageType::LeaveReply
            | ProtoSessionMessageType::GroupAdd
            | ProtoSessionMessageType::GroupRemove
            | ProtoSessionMessageType::GroupWelcome
            | ProtoSessionMessageType::GroupProposal
            | ProtoSessionMessageType::GroupAck => {
                debug!("Skipping channel messages type in encryption path");
                return Ok(());
            }
            _ => {}
        }

        let payload = &msg.get_payload().unwrap().as_application_payload()?.blob;

        let mut mls_guard = self.mls.lock().await;

        debug!("Encrypting message for group member");
        let binding = mls_guard.encrypt_message(payload).await;
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

        msg.set_payload(ApplicationPayload::new("", encrypted_payload.to_vec()).as_content());

        Ok(())
    }

    async fn on_msg_from_slim(&self, msg: &mut Message) -> Result<(), SessionError> {
        // Only process Publish message types
        if !msg.is_publish() {
            debug!("Skipping non-Publish message type in decryption path");
            return Ok(());
        }

        match msg.get_session_header().session_message_type() {
            ProtoSessionMessageType::DiscoveryRequest
            | ProtoSessionMessageType::DiscoveryReply
            | ProtoSessionMessageType::JoinRequest
            | ProtoSessionMessageType::JoinReply
            | ProtoSessionMessageType::LeaveRequest
            | ProtoSessionMessageType::LeaveReply
            | ProtoSessionMessageType::GroupAdd
            | ProtoSessionMessageType::GroupRemove
            | ProtoSessionMessageType::GroupWelcome
            | ProtoSessionMessageType::GroupProposal
            | ProtoSessionMessageType::GroupAck => {
                debug!("Skipping channel messages type in decryption path");
                return Ok(());
            }
            _ => {}
        }

        let payload = &msg.get_payload().unwrap().as_application_payload()?.blob;

        let decrypted_payload = {
            let mut mls_guard = self.mls.lock().await;

            debug!("Decrypting message for group member");
            match mls_guard.decrypt_message(payload).await {
                Ok(decrypted_payload) => decrypted_payload,
                Err(e) => {
                    error!("Failed to decrypt message with MLS: {}", e);
                    return Err(SessionError::MlsDecryptionFailed(e.to_string()));
                }
            }
        };

        msg.set_payload(ApplicationPayload::new("", decrypted_payload.to_vec()).as_content());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_auth::shared_secret::SharedSecret;
    use slim_testing::utils::TEST_VALID_SECRET;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_mls_interceptor_without_group() {
        let mut mls = Mls::new(
            SharedSecret::new("test", TEST_VALID_SECRET),
            SharedSecret::new("test", TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/mls_interceptor_test_without_group"),
        );
        mls.initialize().await.unwrap();

        let mls_arc = Arc::new(Mutex::new(mls));
        let interceptor = MlsInterceptor::new(mls_arc);

        let mut msg = Message::builder()
            .source(
                slim_datapath::messages::Name::from_strings(["org", "default", "test"]).with_id(0),
            )
            .destination(slim_datapath::messages::Name::from_strings([
                "org", "default", "target",
            ]))
            .application_payload("text", b"test message".to_vec())
            .build_publish()
            .unwrap();

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
        let mut alice_mls = Mls::new(
            SharedSecret::new("alice", TEST_VALID_SECRET),
            SharedSecret::new("alice", TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/mls_interceptor_test_alice"),
        );
        let mut bob_mls = Mls::new(
            SharedSecret::new("bob", TEST_VALID_SECRET),
            SharedSecret::new("bob", TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/mls_interceptor_test_bob"),
        );

        alice_mls.initialize().await.unwrap();
        bob_mls.initialize().await.unwrap();

        let _group_id = alice_mls.create_group().await.unwrap();
        let bob_key_package = bob_mls.generate_key_package().await.unwrap();
        let ret = alice_mls.add_member(&bob_key_package).await.unwrap();
        bob_mls.process_welcome(&ret.welcome_message).await.unwrap();

        let alice_interceptor = MlsInterceptor::new(Arc::new(Mutex::new(alice_mls)));
        let bob_interceptor = MlsInterceptor::new(Arc::new(Mutex::new(bob_mls)));

        let original_payload = b"Hello from Alice!";

        let mut alice_msg = Message::builder()
            .source(
                slim_datapath::messages::Name::from_strings(["org", "default", "alice"]).with_id(0),
            )
            .destination(slim_datapath::messages::Name::from_strings([
                "org", "default", "bob",
            ]))
            .application_payload("text", original_payload.to_vec())
            .build_publish()
            .unwrap();

        alice_interceptor
            .on_msg_from_app(&mut alice_msg)
            .await
            .unwrap();

        assert_ne!(
            alice_msg
                .get_payload()
                .unwrap()
                .as_application_payload()
                .unwrap()
                .blob,
            original_payload
        );

        let mut bob_msg = alice_msg.clone();
        bob_interceptor
            .on_msg_from_slim(&mut bob_msg)
            .await
            .unwrap();

        assert_eq!(
            bob_msg
                .get_payload()
                .unwrap()
                .as_application_payload()
                .unwrap()
                .blob,
            original_payload
        );
    }
}
