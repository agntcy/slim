// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Group authentication for node registration.
//!
//! The [`GroupAuthenticator`] enum defines how the control plane verifies
//! that a node is authorized to join its claimed group. The default
//! [`Noop`](GroupAuthenticator::Noop) variant accepts all registrations
//! (backward-compatible behavior when no auth is configured).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use slim_auth::auth_provider::AuthVerifier;
use slim_auth::traits::Verifier;
use tonic::Status;

/// Shared map of group name → verifier. Used by both southbound (read) and
/// northbound (write) services concurrently.
pub type SharedVerifiers = Arc<RwLock<HashMap<String, AuthVerifier>>>;

/// Verifies that a node's credentials authorize it to join a group.
#[derive(Clone, Default)]
pub enum GroupAuthenticator {
    /// Accepts all registrations unconditionally.
    #[default]
    Noop,
    /// Per-group shared secret verification.
    SharedSecret {
        /// Shared map of group name → verifier (built from the per-group secret).
        verifiers: SharedVerifiers,
    },
    /// SPIRE JWT SVID verification.
    /// Validates the JWT SVID against trust bundles provided by the local SPIRE agent
    /// (works with both centralized nested SPIRE and federated deployments).
    /// Convention: trust domain = group name (each cluster has its own trust domain).
    #[cfg(not(target_family = "windows"))]
    Spire {
        /// Verifier that validates JWT SVIDs against available trust bundles.
        /// Boxed because AuthVerifier is ~512 bytes (inline crypto state), while
        /// the other variants are much smaller on the stack (HashMap is a thin pointer).
        verifier: Box<AuthVerifier>,
    },
}

impl std::fmt::Debug for GroupAuthenticator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Noop => write!(f, "GroupAuthenticator::Noop"),
            Self::SharedSecret { verifiers } => f
                .debug_struct("GroupAuthenticator::SharedSecret")
                .field("groups", &verifiers.read().keys().collect::<Vec<_>>())
                .finish(),
            #[cfg(not(target_family = "windows"))]
            Self::Spire { .. } => write!(f, "GroupAuthenticator::Spire"),
        }
    }
}

impl GroupAuthenticator {
    /// Verify that `credentials` prove membership in `claimed_group`.
    ///
    /// For `Noop`, always succeeds (credentials are ignored).
    /// For `SharedSecret`/`Spire`, credentials must be non-empty and valid.
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
            _ if credentials.is_empty() => Err(Status::permission_denied(
                "credentials required but not provided",
            )),
            GroupAuthenticator::SharedSecret { verifiers } => {
                let verifier_clone = {
                    let map = verifiers.read();
                    map.get(claimed_group).cloned()
                };
                let verifier = verifier_clone.ok_or_else(|| {
                    Status::permission_denied(format!(
                        "no auth configured for group '{claimed_group}'"
                    ))
                })?;

                // Verify HMAC and extract claims (includes "sub" = token id).
                let claims: serde_json::Value =
                    verifier.try_get_claims(credentials).map_err(|e| {
                        Status::permission_denied(format!("token verification failed: {e}"))
                    })?;

                // The "sub" field contains "group/node-id" or "group/node-id_RANDOM_SUFFIX".
                // Use exact match or "_" separator to prevent prefix impersonation
                // (e.g., node-10 impersonating node-1).
                let sub = claims.get("sub").and_then(|v| v.as_str()).unwrap_or("");
                let expected_prefix = format!("{claimed_group}/{node_id}");
                let valid =
                    sub == expected_prefix || sub.starts_with(&format!("{expected_prefix}_"));
                if !valid {
                    return Err(Status::permission_denied(format!(
                        "token identity mismatch: expected '{expected_prefix}[_suffix]', got '{sub}'"
                    )));
                }

                Ok(())
            }
            #[cfg(not(target_family = "windows"))]
            GroupAuthenticator::Spire { verifier } => {
                // Verify JWT SVID and extract claims (includes "sub" = SPIFFE ID).
                let claims: serde_json::Value =
                    verifier.try_get_claims(credentials).map_err(|e| {
                        Status::permission_denied(format!("JWT SVID verification failed: {e}"))
                    })?;

                // Extract trust domain from the "sub" claim (SPIFFE ID).
                // Convention: trust_domain == group_name.
                let sub = claims
                    .get("sub")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| Status::permission_denied("missing 'sub' claim in JWT SVID"))?;

                let trust_domain = extract_trust_domain_from_spiffe_id(sub).map_err(|e| {
                    Status::permission_denied(format!("failed to extract trust domain: {e}"))
                })?;

                if trust_domain != claimed_group {
                    return Err(Status::permission_denied(format!(
                        "trust domain '{trust_domain}' does not match claimed group '{claimed_group}'"
                    )));
                }

                Ok(())
            }
        }
    }

    /// Returns `true` if this is the `SharedSecret` variant.
    pub fn is_shared_secret(&self) -> bool {
        matches!(self, Self::SharedSecret { .. })
    }

    /// Add a verifier for a group (shared secret mode only).
    /// Builds the verifier from the raw secret string and inserts it.
    /// Returns an error if this is not the `SharedSecret` variant.
    pub fn add_verifier(&self, group_name: &str, secret: &str) -> Result<(), Status> {
        match self {
            Self::SharedSecret { verifiers } => {
                let verifier =
                    AuthVerifier::shared_secret_from_str(group_name, secret).map_err(|e| {
                        Status::internal(format!(
                            "failed to build verifier for group '{group_name}': {e}"
                        ))
                    })?;
                verifiers.write().insert(group_name.to_string(), verifier);
                Ok(())
            }
            _ => Err(Status::unimplemented(
                "add_verifier is only supported for shared_secret auth",
            )),
        }
    }

    /// Remove the verifier for a group (shared secret mode only).
    pub fn remove_verifier(&self, group_name: &str) {
        if let Self::SharedSecret { verifiers } = self {
            verifiers.write().remove(group_name);
        }
    }
}

/// Extract the trust domain from a SPIFFE ID string.
/// SPIFFE ID format: `spiffe://<trust_domain>/path/...`
#[cfg(not(target_family = "windows"))]
fn extract_trust_domain_from_spiffe_id(spiffe_id: &str) -> Result<String, crate::error::Error> {
    if !spiffe_id.starts_with("spiffe://") {
        return Err(crate::error::Error::InvalidSpiffeId {
            spiffe_id: spiffe_id.to_string(),
            reason: "missing spiffe:// prefix",
        });
    }

    let without_scheme = &spiffe_id["spiffe://".len()..];
    let trust_domain =
        without_scheme
            .split('/')
            .next()
            .ok_or_else(|| crate::error::Error::InvalidSpiffeId {
                spiffe_id: spiffe_id.to_string(),
                reason: "cannot extract trust domain",
            })?;

    if trust_domain.is_empty() {
        return Err(crate::error::Error::InvalidSpiffeId {
            spiffe_id: spiffe_id.to_string(),
            reason: "empty trust domain",
        });
    }

    Ok(trust_domain.to_string())
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
        GroupAuthenticator::SharedSecret {
            verifiers: Arc::new(RwLock::new(verifiers)),
        }
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

    // ── SPIRE trust domain extraction tests ──────────────────────────────

    #[tokio::test]
    async fn shared_secret_rejects_prefix_collision() {
        // Security: node-10's token must NOT pass verification for node-1
        // (prefix "cluster-a/node-1" is a prefix of "cluster-a/node-10_XYZ")
        let auth = make_shared_secret_authenticator("cluster-a", TEST_SECRET);
        let provider =
            AuthProvider::shared_secret_from_str("cluster-a/node-10", TEST_SECRET).unwrap();
        let token = provider.get_token().unwrap();

        let result = auth
            .verify_group_membership(&token, "cluster-a", "node-1")
            .await;
        assert!(result.is_err(), "node-10 must not impersonate node-1");
        assert_eq!(result.unwrap_err().code(), tonic::Code::PermissionDenied);
    }

    #[cfg(not(target_family = "windows"))]
    #[test]
    fn extract_trust_domain_valid_spiffe_id() {
        let td = extract_trust_domain_from_spiffe_id(
            "spiffe://cluster-a.mc-demo.dev.eticloud.io/ns/slim/sa/node",
        )
        .unwrap();
        assert_eq!(td, "cluster-a.mc-demo.dev.eticloud.io");
    }

    #[cfg(not(target_family = "windows"))]
    #[test]
    fn extract_trust_domain_rejects_non_spiffe() {
        let result = extract_trust_domain_from_spiffe_id("not-a-spiffe-id");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing spiffe:// prefix")
        );
    }

    #[cfg(not(target_family = "windows"))]
    #[test]
    fn extract_trust_domain_rejects_empty_domain() {
        let result = extract_trust_domain_from_spiffe_id("spiffe:///path/only");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("empty trust domain")
        );
    }
}
