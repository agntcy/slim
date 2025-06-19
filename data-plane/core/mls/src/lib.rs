// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod errors;
pub mod identity;
pub mod interceptor;
pub mod mls;

use crate::identity::FileBasedIdentityProvider;
use crate::interceptor::MlsInterceptor;
use crate::mls::Mls;
use parking_lot::Mutex;
use std::sync::Arc;

pub async fn add_mls_to_session(
    service: &slim_service::Service,
    agent: &slim_datapath::messages::Agent,
    session_id: slim_service::session::Id,
    participant_id: String,
    identity_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let identity_provider = Arc::new(FileBasedIdentityProvider::new(identity_path)?);

    let mut mls = Mls::new(participant_id, identity_provider);
    mls.initialize().await?;
      
    let interceptor = MlsInterceptor::new(Arc::new(Mutex::new(mls)));

    service.add_session_interceptor(agent, session_id, Box::new(interceptor)).await?;

    Ok(())
}
