// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Third-party crates
use tracing::debug;

use slim_datapath::api::ProtoSessionMessageType;
use slim_datapath::api::{ApplicationPayload, ProtoMessage as Message};
use slim_mls::mls::Mls;

// Local crate
use crate::errors::SessionError;

/// Checks if a message should be processed by MLS encryption/decryption
fn should_process_message(msg: &Message) -> bool {
    // Only process Publish message types
    if !msg.is_publish() {
        debug!("Skipping non-Publish message type");
        return false;
    }

    // Only process actual application data messages (Msg type)
    // Skip all control/session management messages
    match msg.get_session_header().session_message_type() {
        ProtoSessionMessageType::Msg => {
            // This is an application data message, process it
            true
        }
        _ => {
            // Skip all other message types (control messages, ACKs, etc.)
            debug!("Skipping non-data message type: {:?}", msg.get_session_header().session_message_type());
            false
        }
    }
}

/// Encrypts a message payload using MLS
///
/// # Arguments
/// * `mls` - Mutable reference to the MLS instance
/// * `msg` - Mutable reference to the message to encrypt
///
/// # Returns
/// * `Ok(())` if encryption succeeds
/// * `Err(SessionError)` if encryption fails or message format is invalid
pub async fn encrypt_message<P, V>(
    mls: &mut Mls<P, V>,
    msg: &mut Message,
) -> Result<(), SessionError>
where
    P: slim_auth::traits::TokenProvider + Send + Sync + Clone + 'static,
    V: slim_auth::traits::Verifier + Send + Sync + Clone + 'static,
{
    if !should_process_message(msg) {
        return Ok(());
    }

    let payload = &msg.get_payload().unwrap().as_application_payload()?.blob;

    debug!("Encrypting message for group member");
    let encrypted_payload = mls.encrypt_message(payload).await?;

    msg.set_payload(ApplicationPayload::new("", encrypted_payload.to_vec()).as_content());

    Ok(())
}

/// Decrypts a message payload using MLS
///
/// # Arguments
/// * `mls` - Mutable reference to the MLS instance
/// * `msg` - Mutable reference to the message to decrypt
///
/// # Returns
/// * `Ok(())` if decryption succeeds
/// * `Err(SessionError)` if decryption fails or message format is invalid
pub async fn decrypt_message<P, V>(
    mls: &mut Mls<P, V>,
    msg: &mut Message,
) -> Result<(), SessionError>
where
    P: slim_auth::traits::TokenProvider + Send + Sync + Clone + 'static,
    V: slim_auth::traits::Verifier + Send + Sync + Clone + 'static,
{
    if !should_process_message(msg) {
        return Ok(());
    }

    let payload = &msg.get_payload().unwrap().as_application_payload()?.blob;

    debug!("Decrypting message for group member");
    let decrypted_payload = mls.decrypt_message(payload).await?;

    msg.set_payload(ApplicationPayload::new("", decrypted_payload.to_vec()).as_content());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_auth::shared_secret::SharedSecret;
    use slim_testing::utils::TEST_VALID_SECRET;

    #[tokio::test]
    async fn test_encrypt_without_group() {
        let mut mls = Mls::new(
            SharedSecret::new("test", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("test", TEST_VALID_SECRET).unwrap(),
            std::path::PathBuf::from("/tmp/mls_helpers_test_encrypt_without_group"),
        );
        mls.initialize().await.unwrap();

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

        let result = encrypt_message(&mut mls, &mut msg).await;
        assert!(result.is_err_and(|e| matches!(e, SessionError::MlsOp(_))));
    }

    #[tokio::test]
    async fn test_encrypt_decrypt_with_group() {
        let mut alice_mls = Mls::new(
            SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
            std::path::PathBuf::from("/tmp/mls_helpers_test_alice"),
        );
        let mut bob_mls = Mls::new(
            SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
            std::path::PathBuf::from("/tmp/mls_helpers_test_bob"),
        );

        alice_mls.initialize().await.unwrap();
        bob_mls.initialize().await.unwrap();

        let _group_id = alice_mls.create_group().await.unwrap();
        let bob_key_package = bob_mls.generate_key_package().await.unwrap();
        let ret = alice_mls.add_member(&bob_key_package).await.unwrap();
        bob_mls.process_welcome(&ret.welcome_message).await.unwrap();

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

        encrypt_message(&mut alice_mls, &mut alice_msg)
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
        decrypt_message(&mut bob_mls, &mut bob_msg)
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

    #[tokio::test]
    async fn test_skip_non_publish_messages() {
        let mut mls = Mls::new(
            SharedSecret::new("test", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("test", TEST_VALID_SECRET).unwrap(),
            std::path::PathBuf::from("/tmp/mls_helpers_test_skip_non_publish"),
        );
        mls.initialize().await.unwrap();
        let _group_id = mls.create_group().await.unwrap();

        // Create a Subscribe message (not Publish)
        let mut msg = Message::builder()
            .source(
                slim_datapath::messages::Name::from_strings(["org", "default", "test"]).with_id(0),
            )
            .destination(slim_datapath::messages::Name::from_strings([
                "org", "default", "target",
            ]))
            .application_payload("text", b"test message".to_vec())
            .build_subscribe()
            .unwrap();

        let original_payload = msg
            .get_payload()
            .unwrap()
            .as_application_payload()
            .unwrap()
            .blob
            .clone();

        // Should not encrypt
        encrypt_message(&mut mls, &mut msg).await.unwrap();

        // Payload should remain unchanged
        assert_eq!(
            msg.get_payload()
                .unwrap()
                .as_application_payload()
                .unwrap()
                .blob,
            original_payload
        );
    }
}