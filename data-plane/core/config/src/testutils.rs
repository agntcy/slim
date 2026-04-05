// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

#[cfg(feature = "native")]
use tonic::{Request, Response, Status};

#[cfg(feature = "native")]
#[rustfmt::skip]
pub mod helloworld;
pub mod tower_service;

#[cfg(feature = "native")]
#[derive(Default)]
pub struct Empty {}

#[cfg(feature = "native")]
impl Empty {
    pub fn new() -> Self {
        Self {}
    }
}

#[cfg(feature = "native")]
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
