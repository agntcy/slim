// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Owner approval: asking a channel's owner, live, whether to allow a
//! participant change requested without a grant.
//!
//! Pluggable (`OwnerApprover`) like grant verification. The default,
//! [`SlimOwnerApprover`], calls the owner's callback name over SLIM itself
//! via slim-rpc -- the owner's endpoint (e.g. its local SHADI instance) is a
//! SLIM participant connecting outbound, so it needs no inbound ports. The
//! wire contract is `ApprovalRequest`/`ApprovalResponse` in the
//! channel-manager proto, under [`APPROVAL_SERVICE`]/[`APPROVAL_METHOD`].

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use prost::Message;
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_datapath::api::ProtoName;
use slim_rpc::Channel;
use slim_service::app::App;
use tracing::debug;

use crate::proto::approval_response::Decision;
use crate::proto::{ApprovalRequest, ApprovalResponse};

/// slim-rpc service name an owner's endpoint registers its approval handler
/// under.
pub const APPROVAL_SERVICE: &str = "channel_manager.proto.v1.ChannelOwnerApproval";
/// slim-rpc method name for the approval call.
pub const APPROVAL_METHOD: &str = "RequestApproval";

/// How long closing a callback session may take. It runs in the background,
/// after the approval itself is decided.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// The owner's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// A grant signed by the owner -- still verified like any other grant
    /// before anything is allowed.
    Granted(Vec<u8>),
    /// The owner declined, with a reason.
    Denied(String),
}

/// Why the owner couldn't be asked, or didn't answer intelligibly. Every
/// variant denies the request -- an unreachable owner is never a yes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalError {
    /// The callback name isn't a valid SLIM name.
    InvalidCallbackName(String),
    /// The call itself failed (owner unreachable, handler error, ...).
    Transport(String),
    /// The owner answered with something that isn't a valid decision.
    MalformedResponse(String),
}

impl fmt::Display for ApprovalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApprovalError::InvalidCallbackName(name) => {
                write!(f, "invalid owner callback name: {name}")
            }
            ApprovalError::Transport(reason) => write!(f, "approval call failed: {reason}"),
            ApprovalError::MalformedResponse(reason) => {
                write!(f, "malformed approval response: {reason}")
            }
        }
    }
}

impl std::error::Error for ApprovalError {}

/// Asks a channel's owner to approve a participant change.
///
/// A deployment supplies its own implementation to `ChannelManagerServer`
/// (see `ChannelManagerServer::with_owner_approver`) to reach owners some
/// other way. Implementations don't need to enforce a timeout: the caller
/// bounds every call and treats running out of time as a denial.
#[async_trait]
pub trait OwnerApprover: Send + Sync {
    /// Asks the owner reachable at `callback_name` to approve `request`.
    async fn request_approval(
        &self,
        callback_name: &str,
        request: ApprovalRequest,
    ) -> Result<ApprovalDecision, ApprovalError>;
}

/// Default [`OwnerApprover`]: a slim-rpc unary call to the owner's callback
/// name.
pub struct SlimOwnerApprover {
    app: Arc<App<AuthProvider, AuthVerifier>>,
    connection_id: Option<u64>,
}

impl SlimOwnerApprover {
    /// `connection_id` selects the uplink to reach the owner on; `None` uses
    /// the app's default.
    pub fn new(app: Arc<App<AuthProvider, AuthVerifier>>, connection_id: Option<u64>) -> Self {
        Self { app, connection_id }
    }
}

#[async_trait]
impl OwnerApprover for SlimOwnerApprover {
    async fn request_approval(
        &self,
        callback_name: &str,
        request: ApprovalRequest,
    ) -> Result<ApprovalDecision, ApprovalError> {
        let name = ProtoName::parse_name(callback_name)
            .map_err(|_| ApprovalError::InvalidCallbackName(callback_name.to_string()))?;

        let channel =
            Channel::new_with_members(self.app.clone(), vec![name], false, self.connection_id)
                .map_err(|e| ApprovalError::Transport(e.to_string()))?;
        let channel = CloseOnDrop(Some(channel));

        let response_bytes: Vec<u8> = channel
            .get()
            .unary(
                APPROVAL_SERVICE,
                APPROVAL_METHOD,
                request.encode_to_vec(),
                None,
                None,
            )
            .await
            .map_err(|e| ApprovalError::Transport(e.to_string()))?;

        let response = ApprovalResponse::decode(response_bytes.as_slice())
            .map_err(|e| ApprovalError::MalformedResponse(e.to_string()))?;

        match response.decision {
            Some(Decision::Grant(grant)) => Ok(ApprovalDecision::Granted(grant)),
            Some(Decision::Denied(reason)) => Ok(ApprovalDecision::Denied(reason)),
            None => Err(ApprovalError::MalformedResponse(
                "response carries no decision".to_string(),
            )),
        }
    }
}

/// Closes a slim-rpc `Channel`'s session when dropped. A `Channel` doesn't
/// close its own session on drop, and the approval future can be dropped
/// mid-call when the caller's timeout fires -- so closing has to happen
/// here, in a spawned task, rather than after the call returns.
struct CloseOnDrop(Option<Channel>);

impl CloseOnDrop {
    fn get(&self) -> &Channel {
        self.0.as_ref().expect("only taken in drop")
    }
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        let Some(channel) = self.0.take() else {
            return;
        };
        // Drop must not panic, and spawning needs a runtime.
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        runtime.spawn(async move {
            if let Err(e) = channel.close(Some(CLOSE_TIMEOUT)).await {
                debug!(error = %e, "failed to close owner approval session");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::ParticipantAction;
    use slim_auth::shared_secret::SharedSecret;
    use slim_config::component::id::{ID, Kind};
    use slim_rpc::{Context, RpcError, Server};
    use slim_service::service::Service;
    use tokio::sync::Mutex;

    const SECRET: &str = "approval-test-shared-secret-0123456789abcdef";

    /// An in-process SLIM service hosting an "owner" app that answers
    /// approval requests, and a "channel manager" app to call it from.
    struct Env {
        service: Arc<Service>,
        server: Arc<Server>,
        cm_app: Arc<App<AuthProvider, AuthVerifier>>,
        received: Arc<Mutex<Vec<ApprovalRequest>>>,
    }

    impl Env {
        /// `answer` is what the owner replies to every approval request.
        async fn new(test_name: &str, answer: ApprovalResponse) -> Self {
            let id = ID::new_with_name(Kind::new("slim").unwrap(), test_name).unwrap();
            let service = Arc::new(Service::new(id));

            let owner_name = ProtoName::from_strings(["org", "ns", "owner"]);
            let secret = SharedSecret::new("owner", SECRET).unwrap();
            let (owner_app, owner_notifications) = service
                .create_app(
                    &owner_name,
                    AuthProvider::shared_secret(secret.clone()),
                    AuthVerifier::shared_secret(secret),
                )
                .unwrap();
            let owner_app = Arc::new(owner_app);
            let server = Arc::new(Server::new(
                owner_app.clone(),
                owner_app.app_name().clone(),
                owner_notifications,
            ));

            let received = Arc::new(Mutex::new(Vec::new()));
            let recorder = received.clone();
            server.register_unary_unary(
                APPROVAL_SERVICE,
                APPROVAL_METHOD,
                move |bytes: Vec<u8>, _ctx: Context| {
                    let recorder = recorder.clone();
                    let answer = answer.clone();
                    async move {
                        let request = ApprovalRequest::decode(bytes.as_slice())
                            .map_err(|e| RpcError::invalid_argument(e.to_string()))?;
                        recorder.lock().await.push(request);
                        Ok(answer.encode_to_vec())
                    }
                },
            );
            let serving = server.clone();
            tokio::spawn(async move {
                let _ = serving.serve().await;
            });
            tokio::time::sleep(Duration::from_millis(50)).await;

            let cm_name = ProtoName::from_strings(["org", "ns", "channel-manager"]);
            let secret = SharedSecret::new("channel-manager", SECRET).unwrap();
            let (cm_app, _) = service
                .create_app(
                    &cm_name,
                    AuthProvider::shared_secret(secret.clone()),
                    AuthVerifier::shared_secret(secret),
                )
                .unwrap();

            Self {
                service,
                server,
                cm_app: Arc::new(cm_app),
                received,
            }
        }

        async fn shutdown(self) {
            self.server.shutdown().await;
            self.service.shutdown().await.unwrap();
        }
    }

    fn approval_request() -> ApprovalRequest {
        ApprovalRequest {
            channel_name: "org/ns/ch1".to_string(),
            participant_name: "org/ns/agent1".to_string(),
            action: ParticipantAction::Add as i32,
            requester: Some("did:key:z6Mkrequester".to_string()),
        }
    }

    #[tokio::test]
    async fn returns_the_owners_grant_over_slim() {
        let env = Env::new(
            "approval-granted",
            ApprovalResponse {
                decision: Some(Decision::Grant(b"signed-grant".to_vec())),
            },
        )
        .await;
        let approver = SlimOwnerApprover::new(env.cm_app.clone(), None);

        let decision = approver
            .request_approval("org/ns/owner", approval_request())
            .await
            .unwrap();

        assert_eq!(
            decision,
            ApprovalDecision::Granted(b"signed-grant".to_vec())
        );
        assert_eq!(*env.received.lock().await, vec![approval_request()]);
        env.shutdown().await;
    }

    #[tokio::test]
    async fn returns_the_owners_denial_over_slim() {
        let env = Env::new(
            "approval-denied",
            ApprovalResponse {
                decision: Some(Decision::Denied("not today".to_string())),
            },
        )
        .await;
        let approver = SlimOwnerApprover::new(env.cm_app.clone(), None);

        let decision = approver
            .request_approval("org/ns/owner", approval_request())
            .await
            .unwrap();

        assert_eq!(decision, ApprovalDecision::Denied("not today".to_string()));
        env.shutdown().await;
    }

    #[tokio::test]
    async fn rejects_a_response_with_no_decision() {
        let env = Env::new("approval-empty", ApprovalResponse { decision: None }).await;
        let approver = SlimOwnerApprover::new(env.cm_app.clone(), None);

        let err = approver
            .request_approval("org/ns/owner", approval_request())
            .await
            .unwrap_err();

        assert!(matches!(err, ApprovalError::MalformedResponse(_)));
        env.shutdown().await;
    }

    #[tokio::test]
    async fn rejects_an_invalid_callback_name_without_calling_out() {
        let env = Env::new(
            "approval-bad-name",
            ApprovalResponse {
                decision: Some(Decision::Grant(vec![])),
            },
        )
        .await;
        let approver = SlimOwnerApprover::new(env.cm_app.clone(), None);

        let err = approver
            .request_approval("not a slim name", approval_request())
            .await
            .unwrap_err();

        assert_eq!(
            err,
            ApprovalError::InvalidCallbackName("not a slim name".to_string())
        );
        assert!(env.received.lock().await.is_empty());
        env.shutdown().await;
    }
}
