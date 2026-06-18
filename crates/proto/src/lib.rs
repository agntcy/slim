// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Consolidated protobuf/gRPC generated code for SLIM.

mod impls;
pub use impls::*;

pub mod dataplane {
    pub mod proto {
        pub mod v1 {
            include!("gen/dataplane.proto.v1.rs");
        }
    }
}

pub mod controller {
    pub mod proto {
        pub mod v1 {
            include!("gen/controller.proto.v1.rs");
        }
    }
}

pub mod controlplane {
    pub mod proto {
        pub mod v1 {
            include!("gen/controlplane.proto.v1.rs");
        }
    }
}

pub mod channel_manager {
    pub mod proto {
        pub mod v1 {
            include!("gen/channel_manager.proto.v1.rs");
        }
    }
}
