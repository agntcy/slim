#[allow(dead_code)]
pub mod controller {
    pub mod proto {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/controller.proto.v1.rs"));
        }
    }
}

#[allow(dead_code)]
pub mod controlplane {
    pub mod proto {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/controlplane.proto.v1.rs"));
        }
    }
}
