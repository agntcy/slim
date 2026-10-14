// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Shared pieces of the checks that hold peers' names to their verified
//! identities.

use slim_auth::identity_binding::IdentityBinding;
use slim_datapath::api::{ProtoMessage as Message, ProtoName};

/// Report a peer whose name isn't bound to its verified identity: log it, and
/// return whether to drop the message, which only `enforce` does.
pub(crate) fn binding_mismatch(
    mode: IdentityBinding,
    what: &str,
    msg: &Message,
    sub: Option<&str>,
) -> bool {
    if mode == IdentityBinding::Off {
        return false;
    }
    let enforced = mode == IdentityBinding::Enforce;
    tracing::warn!(
        source = %msg.get_source(),
        message_type = ?msg.get_session_message_type(),
        sub = ?sub,
        enforced,
        "{what}"
    );
    enforced
}

/// The same full name: readable components and id.
pub(crate) fn same_name(a: &ProtoName, b: &ProtoName) -> bool {
    a.match_prefix(b) && a.id() == b.id()
}
