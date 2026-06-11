// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use display_error_chain::ErrorChainExt;
use tokio_stream::{Stream, StreamExt};
use tonic::Status;
use tracing::{debug, error};

use crate::api::ProtoMessage;
use crate::api::proto::dataplane::v1::Message;
use crate::connection::Connection;
use crate::errors::DataPathError;
use crate::link_ecdh;
use crate::link_ecdh::X25519_PUBLIC_KEY_LEN;

fn local_version() -> &'static str {
    slim_version::version()
}

/// Perform the mandatory link negotiation phase on a remote connection.
///
/// After this function returns successfully the connection has:
/// - `remote_slim_version` set
/// - `link_id` set (server gets it from the peer; client already has it)
/// - `header_hmac` installed (if ECDH keys are exchanged)
///
/// For the **server** path, a reply is sent to the peer.
pub(crate) async fn run_negotiation<S>(
    connection: &mut Connection,
    stream: &mut S,
) -> Result<(), DataPathError>
where
    S: Stream<Item = Result<Message, Status>> + Unpin + Send,
{
    let is_client = connection.is_outgoing();
    let strict = connection.require_header_mac();

    // Client side: initiate the negotiation by sending the request.
    // The ECDH private key is kept local — it's only needed to derive the
    // HMAC when the reply arrives.
    let mut client_ecdh_private = None;

    if is_client {
        let (sk, pk) = link_ecdh::generate_x25519_ephemeral().map_err(|_| {
            DataPathError::NegotiationError(
                "failed to generate ECDH key pair for link negotiation".to_string(),
            )
        })?;
        client_ecdh_private = Some(sk);

        let link_id = connection.link_id().unwrap_or_default();

        let negotiation_msg = ProtoMessage::builder().build_link_negotiation(
            &link_id,
            local_version(),
            false,
            Some(pk),
        );
        connection.send(negotiation_msg).await.map_err(|e| {
            DataPathError::NegotiationError(format!(
                "failed to send link negotiation request: {}",
                e.chain()
            ))
        })?;
    }

    // Both sides: wait for the negotiation message (request for server, reply for client).
    let msg = match stream.next().await {
        Some(Ok(msg)) => msg,
        Some(Err(e)) => {
            return Err(DataPathError::NegotiationError(format!(
                "error reading link negotiation message: {}",
                e.chain()
            )));
        }
        None => {
            return Err(DataPathError::NegotiationError(
                "stream closed before link negotiation".to_string(),
            ));
        }
    };

    if !msg.is_link() {
        return Err(DataPathError::NegotiationError(
            "first message on remote link is not a link negotiation".to_string(),
        ));
    }

    // Extract the link negotiation payload.
    let payload = msg.get_link_negotiation_payload().ok_or_else(|| {
        DataPathError::NegotiationError("missing link negotiation payload".to_string())
    })?;

    let link_id = &payload.link_id;
    let remote_version = &payload.slim_version;

    debug!(
        %link_id,
        %remote_version,
        is_reply = payload.is_reply,
        is_client,
        "received link negotiation",
    );

    // Role check: clients must only receive replies; servers must only receive requests.
    match (is_client, payload.is_reply) {
        (true, false) => {
            return Err(DataPathError::NegotiationError(
                "received link negotiation request on outgoing connection".to_string(),
            ));
        }
        (false, true) => {
            return Err(DataPathError::NegotiationError(
                "received link negotiation reply on incoming connection".to_string(),
            ));
        }
        _ => {}
    }

    // Parse the remote version.
    let version = semver::Version::parse(remote_version).map_err(|e| {
        DataPathError::NegotiationError(format!("unparsable remote SLIM version: {}", e))
    })?;

    if is_client {
        // Client path: validate the reply and derive HMAC.
        if strict && payload.link_ecdh_public_key.len() != X25519_PUBLIC_KEY_LEN {
            return Err(DataPathError::NegotiationError(
                "public key length is invalid".to_string(),
            ));
        }

        if !connection.complete_negotiation_as_client(link_id, version) {
            return Err(DataPathError::NegotiationError(
                "link negotiation reply rejected (link_id mismatch or replay)".to_string(),
            ));
        }

        if payload.link_ecdh_public_key.len() == X25519_PUBLIC_KEY_LEN
            && let Some(sk) = client_ecdh_private.take()
        {
            match link_ecdh::derive_header_mac_from_ecdh(
                sk,
                payload.link_ecdh_public_key.as_slice(),
                link_id,
            ) {
                Ok(mac) => connection.install_header_hmac(mac),
                Err(e) => {
                    error!(
                        error = %e,
                        "link ECDH key derivation failed (client path)",
                    );
                    return Err(DataPathError::NegotiationError(
                        "failed to generate client exchange key".to_string(),
                    ));
                }
            }
        }

        if strict && connection.header_hmac().is_none() {
            return Err(DataPathError::NegotiationError(
                "strict header MAC required but link HMAC session is not installed".to_string(),
            ));
        }
    } else {
        // Server path: validate the request, derive HMAC, send reply.
        if strict && payload.link_ecdh_public_key.len() != X25519_PUBLIC_KEY_LEN {
            return Err(DataPathError::NegotiationError(
                "public key length is invalid".to_string(),
            ));
        }

        if !connection.complete_negotiation_as_server(link_id, version) {
            return Err(DataPathError::NegotiationError(
                "link negotiation request rejected (empty link_id or replay)".to_string(),
            ));
        }

        let peer_ecdh = payload.link_ecdh_public_key.as_slice();
        let mut server_reply_ecdh: Option<Vec<u8>> = None;
        if peer_ecdh.len() == X25519_PUBLIC_KEY_LEN {
            match link_ecdh::generate_x25519_ephemeral() {
                Ok((server_sk, server_pk)) => {
                    match link_ecdh::derive_header_mac_from_ecdh(server_sk, peer_ecdh, link_id) {
                        Ok(mac) => {
                            connection.install_header_hmac(mac);
                            server_reply_ecdh = Some(server_pk);
                        }
                        Err(e) => {
                            error!(
                                error = %e,
                                "link ECDH key derivation failed (server path)",
                            );
                            if strict {
                                return Err(DataPathError::NegotiationError(
                                    "failed to derive header MAC from link ECDH (server path)"
                                        .to_string(),
                                ));
                            }
                        }
                    }
                }
                Err(_) => {
                    return Err(DataPathError::NegotiationError(
                        "failed to generate server exchange key".to_string(),
                    ));
                }
            }
        }

        if strict && connection.header_hmac().is_none() {
            return Err(DataPathError::NegotiationError(
                "strict header MAC required but link HMAC session is not installed".to_string(),
            ));
        }

        // Send reply.
        let reply = ProtoMessage::builder().build_link_negotiation(
            link_id,
            local_version(),
            true,
            server_reply_ecdh,
        );
        connection.send(reply).await.map_err(|e| {
            DataPathError::NegotiationError(format!(
                "failed to send link negotiation reply: {}",
                e.chain()
            ))
        })?;
    }

    debug!("link negotiation completed successfully");
    Ok(())
}
