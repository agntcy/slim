// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_datapath::messages::Name as SlimName;

/// Name type for SLIM (Secure Low-Latency Interactive Messaging)
#[derive(Debug, Clone, PartialEq, uniffi::Object)]
pub struct Name {
    inner: SlimName,
}

impl From<Name> for SlimName {
    fn from(name: Name) -> Self {
        name.inner.clone()
    }
}

impl From<SlimName> for Name {
    fn from(name: SlimName) -> Self {
        Name {
            inner: name,
        }
    }
}

impl From<&SlimName> for Name {
    fn from(name: &SlimName) -> Self {
        Name {
            inner: name.clone(),
        }
    }
}
