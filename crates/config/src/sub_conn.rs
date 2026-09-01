// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Sub-connection selection policy for logical connections.
//!
//! A logical connection groups every physical connection that terminates on the
//! same remote SLIM node. When the datapath has a message to send to that node
//! it must pick one (or more) of the grouped sub-connections; [`SubConnPolicy`]
//! is what decides.
//!
//! Kept transport-free (like [`crate::conn_type::ConnType`]) so it is available
//! on `wasm32-unknown-unknown` as well.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How a logical connection picks among its sub-connections for each message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubConnPolicy {
    /// Send each message on the next healthy sub-connection, striping load
    /// across all of them. Raises throughput past a single connection's
    /// flow-control ceiling but does not preserve message order across
    /// sub-connections.
    #[default]
    RoundRobin,

    /// Send every message on all sub-connections. Delivery succeeds if any one
    /// of them succeeds, trading bandwidth for a reliability bound without
    /// paying the round-trip cost of an application-level retry.
    ///
    /// The `1 - p^N` bound only holds when the sub-connections do not share a
    /// failure domain (separate TCP connections, ideally separate paths).
    Redundant,

    /// Pin a flow to one sub-connection by hashing its affinity key (session
    /// id). Stripes across flows while preserving ordering within each one.
    Affinity,

    /// Always use the first healthy sub-connection, moving to the next only
    /// when it goes away. Resilience with no duplicate traffic.
    Failover,
}

impl SubConnPolicy {
    /// Whether this policy sends a copy of the message on every sub-connection.
    pub fn is_redundant(self) -> bool {
        matches!(self, SubConnPolicy::Redundant)
    }
}

impl std::fmt::Display for SubConnPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SubConnPolicy::RoundRobin => "round_robin",
            SubConnPolicy::Redundant => "redundant",
            SubConnPolicy::Affinity => "affinity",
            SubConnPolicy::Failover => "failover",
        };
        f.write_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_round_robin() {
        assert_eq!(SubConnPolicy::default(), SubConnPolicy::RoundRobin);
    }

    #[test]
    fn only_redundant_duplicates() {
        assert!(SubConnPolicy::Redundant.is_redundant());
        assert!(!SubConnPolicy::RoundRobin.is_redundant());
        assert!(!SubConnPolicy::Affinity.is_redundant());
        assert!(!SubConnPolicy::Failover.is_redundant());
    }

    #[test]
    fn serde_roundtrip_snake_case() {
        for (policy, json) in [
            (SubConnPolicy::RoundRobin, "\"round_robin\""),
            (SubConnPolicy::Redundant, "\"redundant\""),
            (SubConnPolicy::Affinity, "\"affinity\""),
            (SubConnPolicy::Failover, "\"failover\""),
        ] {
            assert_eq!(serde_json::to_string(&policy).unwrap(), json);
            assert_eq!(serde_json::from_str::<SubConnPolicy>(json).unwrap(), policy);
        }
    }

    #[test]
    fn display_matches_serde() {
        for policy in [
            SubConnPolicy::RoundRobin,
            SubConnPolicy::Redundant,
            SubConnPolicy::Affinity,
            SubConnPolicy::Failover,
        ] {
            let via_serde = serde_json::to_string(&policy).unwrap();
            assert_eq!(format!("\"{policy}\""), via_serde);
        }
    }
}
