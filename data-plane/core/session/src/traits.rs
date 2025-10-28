// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Third-party crates

use slim_datapath::Status;
use slim_datapath::api::ProtoMessage as Message;

// Local crate
use super::SessionError;
use super::SessionInterceptorProvider;

/// Session transmitter trait
pub trait Transmitter: SessionInterceptorProvider {
    fn send_to_slim(
        &self,
        message: Result<Message, Status>,
    ) -> impl Future<Output = Result<(), SessionError>> + Send + 'static;

    fn send_to_app(
        &self,
        message: Result<Message, SessionError>,
    ) -> impl Future<Output = Result<(), SessionError>> + Send + 'static;
}
