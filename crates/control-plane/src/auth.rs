// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Group authentication for node registration.
//!
//! The [`GroupAuthenticator`] enum defines how the control plane verifies
//! that a node is authorized to join its claimed group. The default
//! [`Noop`](GroupAuthenticator::Noop) variant accepts all registrations
//! (backward-compatible behavior when no auth is configured).

use tonic::Status;

/// Verifies that a node's credentials authorize it to join a group.
#[derive(Debug, Clone)]
pub enum GroupAuthenticator {
    /// Accepts all registrations unconditionally.
    Noop,
    // Future variants:
    // SharedSecret { groups: HashMap<String, ...> },
    // Spire { socket_path: String },
}

impl GroupAuthenticator {
    /// Verify that `credentials` prove membership in `claimed_group`.
    ///
    /// Returns `Ok(())` on success, or a `PERMISSION_DENIED` status on failure.
    pub async fn verify_group_membership(
        &self,
        _credentials: &str,
        _claimed_group: &str,
    ) -> Result<(), Status> {
        match self {
            GroupAuthenticator::Noop => Ok(()),
        }
    }
}
