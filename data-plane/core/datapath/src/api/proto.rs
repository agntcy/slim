// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod dataplane {
    pub mod v1 {
        // The size difference between Publish and Subscribe/Unsubscribe message
        // is too large for clippy, so we allow it here.
        #![allow(clippy::large_enum_variant)]
        include!("gen/dataplane.proto.v1.rs");
    }
}
