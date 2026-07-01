// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Group authentication for node registration.
//!
//! The [`GroupAuthenticator`] enum defines how the control plane verifies
//! that a node is authorized to join its claimed group. The default
//! [`Noop`](GroupAuthenticator::Noop) variant accepts all registrations
//! (backward-compatible behavior when no auth is configured).

use std::collections::HashMap;

use slim_auth::auth_provider::AuthVerifier;
use slim_auth::traits::Verifier;
use tonic::Status;

/// Verifies that a node's credentials authorize it to join a group.
#[derive(Clone)]
pub enum GroupAuthenticator {
    /// Accepts all registrations unconditionally.
    Noop,
    /// Per-group shared secret verification.
    SharedSecret {
        /// Map of group name → verifier (built from the per-group secret).
        verifiers: HashMap<String, AuthVerifier>,
    },
    // Future: Spire { ... },
}

impl std::fmt::Debug for GroupAuthenticator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Noop => write!(f, "GroupAuthenticator::Noop"),
            Self::SharedSecret { verifiers } => f
                .debug_struct("GroupAuthenticator::SharedSecret")
                .field("groups", &verifiers.keys().collect::<Vec<_>>())
                .finish(),
        }
    }
}

impl GroupAuthenticator {
    /// Verify that `credentials` prove membership in `claimed_group`.
    ///
    /// Returns `Ok(())` on success, or a `PERMISSION_DENIED` status on failure.
    pub async fn verify_group_membership(
        &self,
        credentials: &str,
        claimed_group: &str,
        node_id: &str,
    ) -> Result<(), Status> {
        match self {
            GroupAuthenticator::Noop => Ok(()),
            GroupAuthenticator::SharedSecret { verifiers } => {
                if credentials.is_empty() {
                    return Err(Status::permission_denied(
                        "credentials required but not provided",
                    ));
                }

                let verifier = verifiers.get(claimed_group).ok_or_else(|| {
                    Status::permission_denied(format!(
                        "no auth configured for group '{claimed_group}'"
                    ))
                })?;

                // Verify HMAC signature (proves the node has the correct secret).
                verifier.try_verify(credentials).map_err(|e| {
                    Status::permission_denied(format!("token verification failed: {e}"))
                })?;

                // Extract the token id (first field) and verify it matches
                // the expected identity: "group/node-id_SUFFIX".
                let token_id = credentials.split(':').next().unwrap_or("");
                let expected_prefix = format!("{claimed_group}/{node_id}");
                if !token_id.starts_with(&expected_prefix) {
                    return Err(Status::permission_denied(format!(
                        "token identity mismatch: expected prefix '{expected_prefix}', got '{token_id}'"
                    )));
                }

                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_auth::auth_provider::AuthProvider;
    use slim_auth::traits::TokenProvider;

    const TEST_SECRET: &str = "test-secret-0123456789-abcdefghijk";

    fn make_shared_secret_authenticator(group: &str, secret: &str) -> GroupAuthenticator {
        let verifier = AuthVerifier::shared_secret_from_str(group, secret).unwrap();
        let mut verifiers = HashMap::new();
        verifiers.insert(group.to_string(), verifier);
        GroupAuthenticator::SharedSecret { verifiers }
    }

    #[tokio::test]
    async fn noop_accepts_everything() {
        let auth = GroupAuthenticator::Noop;
        assert!(
            auth.verify_group_membership("", "any-group", "any-node")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn shared_secret_accepts_valid_token() {
        let auth = make_shared_secret_authenticator("cluster-a", TEST_SECRET);
        // Node creates a provider with id = "cluster-a/node-1"
        let provider =
            AuthProvider::shared_secret_from_str("cluster-a/node-1", TEST_SECRET).unwrap();
        let token = provider.get_token().unwrap();

        let result = auth
            .verify_group_membership(&token, "cluster-a", "node-1")
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn shared_secret_rejects_wrong_secret() {
        let auth = make_shared_secret_authenticator("cluster-a", TEST_SECRET);
        let provider = AuthProvider::shared_secret_from_str(
            "cluster-a/node-1",
            "different-secret-0123456789abcdef",
        )
        .unwrap();
        let token = provider.get_token().unwrap();

        let result = auth
            .verify_group_membership(&token, "cluster-a", "node-1")
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::PermissionDenied);
    }

    #[tokio::test]
    async fn shared_secret_rejects_empty_credentials() {
        let auth = make_shared_secret_authenticator("cluster-a", TEST_SECRET);

        let result = auth
            .verify_group_membership("", "cluster-a", "node-1")
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::PermissionDenied);
    }

    #[tokio::test]
    async fn shared_secret_rejects_unknown_group() {
        let auth = make_shared_secret_authenticator("cluster-a", TEST_SECRET);
        let provider =
            AuthProvider::shared_secret_from_str("cluster-b/node-1", TEST_SECRET).unwrap();
        let token = provider.get_token().unwrap();

        let result = auth
            .verify_group_membership(&token, "cluster-b", "node-1")
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::PermissionDenied);
    }

    #[tokio::test]
    async fn shared_secret_rejects_wrong_node_id() {
        let auth = make_shared_secret_authenticator("cluster-a", TEST_SECRET);
        // Token is for node-1 but claiming to be node-2
        let provider =
            AuthProvider::shared_secret_from_str("cluster-a/node-1", TEST_SECRET).unwrap();
        let token = provider.get_token().unwrap();

        let result = auth
            .verify_group_membership(&token, "cluster-a", "node-2")
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::PermissionDenied);
    }

    #[tokio::test]
    async fn shared_secret_rejects_wrong_group_in_token() {
        let auth = make_shared_secret_authenticator("cluster-a", TEST_SECRET);
        // Token id says cluster-b but trying to register in cluster-a
        let provider =
            AuthProvider::shared_secret_from_str("cluster-b/node-1", TEST_SECRET).unwrap();
        let token = provider.get_token().unwrap();

        let result = auth
            .verify_group_membership(&token, "cluster-a", "node-1")
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::PermissionDenied);
    }
}
