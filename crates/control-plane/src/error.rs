// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::node_transport::{NodeStatus, ResponseKind};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    // ── node_transport ─────────────────────────────────────────────────────────
    /// Caller passed an empty node ID.
    #[error("node ID cannot be empty")]
    EmptyNodeId,

    /// Caller passed an empty message ID.
    #[error("message ID cannot be empty")]
    EmptyMessageId,

    /// The target node has no active CP connection.
    #[error("node {node_id} is not connected (status: {status})")]
    NodeNotConnected { node_id: String, status: NodeStatus },

    /// No outbound stream is registered for the node.
    #[error("no stream found for node {node_id}")]
    StreamNotFound { node_id: String },

    /// The mpsc channel send to the node's stream failed.
    #[error("failed to send to node {node_id}: {reason}")]
    SendFailed { node_id: String, reason: String },

    /// No reply arrived within the deadline.
    #[error("timeout waiting for {kind:?} response from node {node_id}")]
    ResponseTimeout { node_id: String, kind: ResponseKind },

    // ── db ────────────────────────────────────────────────────────────────────
    #[error("node {id} not found")]
    NodeNotFound { id: String },

    #[error("route {id} already exists")]
    RouteAlreadyExists { id: String },

    #[error("route {id} not found")]
    RouteNotFound { id: String },

    #[error("link {id} not found")]
    LinkNotFound { id: String },

    /// One or more required link identity fields are empty.
    #[error("link fields cannot be empty")]
    LinkMissingFields,

    /// An underlying database operation failed (pool error, query error, etc.)
    #[error("db error in {context}: {msg}")]
    DbError { context: &'static str, msg: String },

    // ── route ─────────────────────────────────────────────────────────────────
    #[error("source node ID cannot be empty")]
    EmptySourceNodeId,

    #[error("destination node ID cannot be empty")]
    EmptyDestNodeId,

    #[error("destination node ID cannot be the same as source node ID")]
    SameSourceAndDest,

    // ── service ───────────────────────────────────────────────────────────────
    /// Invalid or missing input (validation failures, business-rule violations).
    #[error("{0}")]
    InvalidInput(String),

    /// The remote node returned an unexpected response type.
    #[error("unexpected response: {0}")]
    UnexpectedResponse(String),

    // ── serialization ────────────────────────────────────────────────────────
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    // ── auth ──────────────────────────────────────────────────────────────────
    /// Invalid SPIFFE ID format.
    #[error("invalid SPIFFE ID '{spiffe_id}': {reason}")]
    InvalidSpiffeId {
        spiffe_id: String,
        reason: &'static str,
    },

    // ── topology ─────────────────────────────────────────────────────────────
    /// A routing policy allows visibility between groups that have no direct
    /// link and no transit path (topology is not star-shaped).
    #[error(
        "routing policy allows '{from}' to reach '{to}' but no direct link exists \
         and topology is not star-shaped (transit not available)"
    )]
    NoLinkForVisibility { from: String, to: String },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
