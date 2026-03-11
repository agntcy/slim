// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

#[cfg(feature = "grpc")]
use tonic::{Request, Response, Status};

#[cfg(feature = "grpc")]
#[rustfmt::skip]
pub mod helloworld;
pub mod tower_service;

#[cfg(feature = "grpc")]
#[derive(Default)]
pub struct Empty {}

#[cfg(feature = "grpc")]
impl Empty {
    pub fn new() -> Self {
        Self {}
    }
}

#[cfg(feature = "grpc")]
#[tonic::async_trait]
impl helloworld::greeter_server::Greeter for Empty {
    async fn say_hello(
        &self,
        request: Request<helloworld::HelloRequest>,
    ) -> Result<Response<helloworld::HelloReply>, Status> {
        let reply = helloworld::HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };

        Ok(Response::new(reply))
    }
}
