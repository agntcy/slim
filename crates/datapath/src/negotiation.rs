// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use display_error_chain::ErrorChainExt;
use tokio_stream::{Stream, StreamExt};
use tonic::Status;
use tracing::{debug, error};

use crate::api::ProtoMessage;
use crate::api::proto::dataplane::v1::{LinkConnectionType, Message};
use crate::connection::Connection;
use crate::errors::DataPathError;
use crate::header_mac::HeaderMacError;
use crate::link_ecdh;
use crate::link_ecdh::{ML_KEM768_CIPHERTEXT_LEN, ML_KEM768_PUBLIC_KEY_LEN, X25519_PUBLIC_KEY_LEN};
use crate::tables::ConnType;

fn local_version() -> &'static str {
    slim_version::version()
}

/// Parameters for running the link negotiation that come from the node's configuration.
pub(crate) struct NegotiationParams<'a> {
    /// This node's identity (e.g. "slim/a").
    pub node_id: &'a str,
    /// Deployment name for peer-group verification.
    pub deployment_name: &'a str,
    /// The connection type the local side wants to advertise to the remote.
    pub connection_type: ConnType,
    /// When true, link negotiation must use hybrid X25519 + ML-KEM-768 key agreement.
    pub enforce_pqc: bool,
}

/// Result of a successful negotiation.
pub(crate) struct NegotiationResult {
    /// The connection type indicated by the remote side.
    pub(crate) connection_type: ConnType,
    /// The remote node's identifier (from the negotiation payload).
    pub(crate) remote_node_id: String,
    /// The remote's advertised deployment name.
    pub(crate) remote_deployment_name: String,
}

/// Perform the mandatory link negotiation phase on a remote connection.
///
/// After this function returns successfully the connection has:
/// - `remote_slim_version` set
/// - `link_id` set (server gets it from the peer; client already has it)
/// - `header_hmac` installed (if ECDH keys are exchanged)
/// - `peer_node_id` set (if provided by the remote)
///
/// For the **server** path, a reply is sent to the peer.
pub(crate) async fn run_negotiation<S>(
    connection: &mut Connection,
    stream: &mut S,
    params: &NegotiationParams<'_>,
) -> Result<NegotiationResult, DataPathError>
where
    S: Stream<Item = Result<Message, Status>> + Unpin + Send,
{
    let is_client = connection.is_outgoing();
    let strict = connection.require_header_mac();

    let mut client_ecdh_private = None;
    let mut client_mlkem_private = None;
    if is_client {
        let (sk, pk) = link_ecdh::generate_x25519_ephemeral().map_err(|_| {
            DataPathError::NegotiationError(
                "failed to generate ECDH key pair for link negotiation".to_string(),
            )
        })?;
        client_ecdh_private = Some(sk);
        let mut kem_payload = None;

        if params.enforce_pqc {
            let (mlkem_sk, mlkem_pk) = link_ecdh::generate_mlkem768().map_err(|_| {
                DataPathError::NegotiationError(
                    "failed to generate ML-KEM-768 key pair for link negotiation".to_string(),
                )
            })?;
            client_mlkem_private = Some(mlkem_sk);
            kem_payload = Some(mlkem_pk);
        }

        let link_id = connection.link_id().unwrap_or_default();

        let negotiation_msg = ProtoMessage::builder().build_link_negotiation(
            &link_id,
            local_version(),
            false,
            Some(pk),
            kem_payload,
            conn_type_to_link(params.connection_type),
            params.node_id,
            params.deployment_name,
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

    if is_client && params.enforce_pqc && kem_len(&payload) != ML_KEM768_CIPHERTEXT_LEN {
        return Err(DataPathError::NegotiationError(
            "enforce_pqc: responder did not return ML-KEM-768 ciphertext".into(),
        ));
    }
    if !is_client && params.enforce_pqc && kem_len(&payload) != ML_KEM768_PUBLIC_KEY_LEN {
        return Err(DataPathError::NegotiationError(
            "enforce_pqc: initiator did not send ML-KEM-768 public key".into(),
        ));
    }

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

    // Capture remote identity from the payload.
    let remote_node_id = payload.node_id.clone();
    let remote_deployment_name = payload.deployment_name.clone();
    let remote_conn_type = match LinkConnectionType::try_from(payload.connection_type) {
        Ok(LinkConnectionType::Peer) => ConnType::Peer,
        Ok(LinkConnectionType::Edge) => ConnType::Edge,
        _ => ConnType::Remote,
    };

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

        // Store remote node identity.
        if !remote_node_id.is_empty() {
            connection.set_peer_node_id(remote_node_id.clone());
        }

        if payload.link_ecdh_public_key.len() == X25519_PUBLIC_KEY_LEN
            && let Some(sk) = client_ecdh_private.take()
        {
            let derive_result = if params.enforce_pqc {
                let ct = payload.link_kem_payload.as_deref().ok_or_else(|| {
                    DataPathError::NegotiationError("missing ML-KEM ciphertext".into())
                })?;
                let mlkem_sk = client_mlkem_private.take().ok_or_else(|| {
                    DataPathError::NegotiationError("missing local ML-KEM secret".into())
                })?;
                let mlkem_shared = link_ecdh::decapsulate_mlkem768(mlkem_sk, ct).map_err(|_| {
                    DataPathError::NegotiationError("ML-KEM decapsulation failed".into())
                })?;
                link_ecdh::derive_header_mac_hybrid(
                    sk,
                    payload.link_ecdh_public_key.as_slice(),
                    &mlkem_shared,
                    link_id,
                )
            } else {
                link_ecdh::derive_header_mac_from_ecdh(
                    sk,
                    payload.link_ecdh_public_key.as_slice(),
                    link_id,
                )
            };

            match derive_result {
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

        if params.enforce_pqc && connection.header_hmac().is_none() {
            return Err(DataPathError::NegotiationError(
                "enforce_pqc: hybrid link HMAC session is not installed".into(),
            ));
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

        // Store remote node identity.
        if !remote_node_id.is_empty() {
            connection.set_peer_node_id(remote_node_id.clone());
        }

        let peer_ecdh = payload.link_ecdh_public_key.as_slice();
        let mut server_reply_ecdh: Option<Vec<u8>> = None;
        let mut reply_kem_payload: Option<Vec<u8>> = None;
        if peer_ecdh.len() == X25519_PUBLIC_KEY_LEN {
            let (server_sk, server_pk) = link_ecdh::generate_x25519_ephemeral().map_err(|_| {
                DataPathError::NegotiationError("failed to generate server exchange key".into())
            })?;

            let hybrid = params.enforce_pqc || kem_len(&payload) == ML_KEM768_PUBLIC_KEY_LEN;
            let derive_result = if hybrid {
                // Use an IIFE so ? propagates into derive_result (-> Err arm of the match
                // below), letting strict=false deployments skip the header MAC gracefully
                // instead of hard-rejecting the connection.
                (|| {
                    let peer_kem_pk = payload
                        .link_kem_payload
                        .as_deref()
                        .ok_or(HeaderMacError::KeyAgreement)?;
                    let (ct, mlkem_shared) = link_ecdh::encapsulate_mlkem768(peer_kem_pk)?;
                    reply_kem_payload = Some(ct);
                    link_ecdh::derive_header_mac_hybrid(server_sk, peer_ecdh, &mlkem_shared, link_id)
                })()
            } else {
                link_ecdh::derive_header_mac_from_ecdh(server_sk, peer_ecdh, link_id)
            };

            match derive_result {
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
                            "failed to derive header MAC from link ECDH (server path)".to_string(),
                        ));
                    }
                }
            }

            if params.enforce_pqc && connection.header_hmac().is_none() {
                return Err(DataPathError::NegotiationError(
                    "enforce_pqc: hybrid link HMAC session is not installed".into(),
                ));
            }
        }

        if strict && connection.header_hmac().is_none() {
            return Err(DataPathError::NegotiationError(
                "strict header MAC required but link HMAC session is not installed".to_string(),
            ));
        }

        // Echo back with the same connection_type the remote requested — this
        // tells the client that the server acknowledges the peer upgrade intent.
        let reply_conn_type = LinkConnectionType::try_from(payload.connection_type)
            .unwrap_or(LinkConnectionType::Remote);

        // Send reply.
        let reply = ProtoMessage::builder().build_link_negotiation(
            link_id,
            local_version(),
            true,
            server_reply_ecdh,
            reply_kem_payload,
            reply_conn_type,
            params.node_id,
            params.deployment_name,
        );
        connection.send(reply).await.map_err(|e| {
            DataPathError::NegotiationError(format!(
                "failed to send link negotiation reply: {}",
                e.chain()
            ))
        })?;
    }

    debug!("link negotiation completed successfully");
    Ok(NegotiationResult {
        connection_type: remote_conn_type,
        remote_node_id,
        remote_deployment_name,
    })
}

/// Map internal [`ConnType`] to the protobuf [`LinkConnectionType`] for the wire.
fn conn_type_to_link(ct: ConnType) -> LinkConnectionType {
    match ct {
        ConnType::Peer => LinkConnectionType::Peer,
        ConnType::Edge => LinkConnectionType::Edge,
        _ => LinkConnectionType::Remote,
    }
}

fn kem_len(payload: &crate::api::LinkNegotiationPayload) -> usize {
    payload.link_kem_payload.as_ref().map_or(0, |b| b.len())
}
