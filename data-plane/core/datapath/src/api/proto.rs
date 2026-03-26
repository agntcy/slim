// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod dataplane {
    pub mod v1 {
        // The generated proto file contains both prost message types (always available)
        // and tonic gRPC service stubs (native-only). We include the full file and
        // rely on the cfg-gated re-exports in api.rs to control visibility.
        include!("gen/dataplane.proto.v1.rs");
    }
}
