// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod dataplane {
    pub mod v1 {
        include!("gen/dataplane.proto.v1.rs");
    }
}
