// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Consolidated protobuf/gRPC generated code for SLIM.

mod impls;
pub use impls::*;

pub mod dataplane {
    pub mod proto {
        #[allow(clippy::large_enum_variant)]
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/dataplane.proto.v1.rs"));
        }
    }
}

pub mod controller {
    pub mod proto {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/controller.proto.v1.rs"));
        }
    }
}

pub mod controlplane {
    pub mod proto {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/controlplane.proto.v1.rs"));
        }
    }
}

pub mod channel_manager {
    pub mod proto {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/channel_manager.proto.v1.rs"));
        }
    }
}
