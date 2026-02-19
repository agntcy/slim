// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// The proto package names are "controlplane.proto.v1" and "controller.proto.v1".
// Prost generates cross-package references using the full package path as module path.
// From within `controlplane::proto::v1`, cross-refs to `controller.proto.v1` go:
// super::super::super::controller::proto::v1 (up 3 levels to crate root, then down).
// So we declare modules matching the package hierarchy at the crate root.

pub mod controlplane {
    pub mod proto {
        #[allow(dead_code)]
        pub mod v1 {
            tonic::include_proto!("controlplane.proto.v1");
        }
    }
}

pub mod controller {
    pub mod proto {
        pub mod v1 {
            tonic::include_proto!("controller.proto.v1");
        }
    }
}
