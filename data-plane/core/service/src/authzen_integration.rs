// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! AuthZEN integration for SLIM service layer
//! 
//! This module provides the bridge between SLIM's internal types and the 
//! OpenID AuthZEN authorization API standard.

use std::time::Duration;

use serde_json::json;
use slim_auth::authzen::{
    AuthZenAction, AuthZenAuthorizer, AuthZenConfig, AuthZenResource, AuthZenSubject,
};
use slim_auth::errors::AuthError;
use slim_datapath::messages::encoder::{Agent, AgentType};
use tracing::{debug, warn};

use crate::errors::ServiceError;

/// Local conversion functions to avoid orphan rule violations
fn agent_to_authzen_subject(agent: &Agent) -> AuthZenSubject {
    let properties = if let (Some(org), Some(ns), Some(agent_type)) = (
        agent.agent_type().organization_string(),
        agent.agent_type().namespace_string(),
        agent.agent_type().agent_type_string(),
    ) {
        Some(json!({
            "organization": org,
            "namespace": ns,
            "agent_type": agent_type,
            "agent_id": agent.agent_id()
        }))
    } else {
        Some(json!({
            "agent_id": agent.agent_id()
        }))
    };

    AuthZenSubject {
        subject_type: "agent".to_string(),
        id: agent.to_string(),
        properties,
    }
}

fn agent_to_authzen_resource(agent: &Agent) -> AuthZenResource {
    let properties = if let (Some(org), Some(ns), Some(agent_type)) = (
        agent.agent_type().organization_string(),
        agent.agent_type().namespace_string(),
        agent.agent_type().agent_type_string(),
    ) {
        Some(json!({
            "organization": org,
            "namespace": ns,
            "agent_type": agent_type,
            "agent_id": agent.agent_id()
        }))
    } else {
        Some(json!({
            "agent_id": agent.agent_id()
        }))
    };

    AuthZenResource {
        resource_type: "agent".to_string(),
        id: agent.to_string(),
        properties,
    }
}

fn agent_type_to_authzen_resource(agent_type: &AgentType) -> AuthZenResource {
    let properties = if let (Some(org), Some(ns), Some(agent_type_str)) = (
        agent_type.organization_string(),
        agent_type.namespace_string(),
        agent_type.agent_type_string(),
    ) {
        Some(json!({
            "organization": org,
            "namespace": ns,
            "agent_type": agent_type_str
        }))
    } else {
        None
    };

    AuthZenResource {
        resource_type: "route".to_string(),
        id: agent_type.to_string(),
        properties,
    }
}

/// AuthZEN service integration wrapper
pub struct AuthZenService {
    authorizer: Option<AuthZenAuthorizer>,
    enabled: bool,
}

impl AuthZenService {
    /// Create a new AuthZEN service integration
    pub fn new(config: Option<AuthZenServiceConfig>) -> Result<Self, ServiceError> {
        let (authorizer, enabled) = match config {
            Some(config) if config.enabled => {
                let authzen_config = AuthZenConfig {
                    pdp_endpoint: config.pdp_endpoint,
                    timeout: config.timeout,
                    cache_ttl: config.cache_ttl,
                    fallback_allow: config.fallback_allow,
                    max_retries: config.max_retries,
                };

                let authorizer = AuthZenAuthorizer::new(authzen_config)
                    .map_err(|e| ServiceError::ConfigError(format!("AuthZEN setup failed: {}", e)))?;

                (Some(authorizer), true)
            }
            _ => (None, false),
        };

        Ok(Self { authorizer, enabled })
    }

    /// Check if AuthZEN is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Authorize route creation
    pub async fn authorize_route(
        &self,
        agent: &Agent,
        target_type: &AgentType,
        connection_id: Option<u64>,
    ) -> Result<bool, ServiceError> {
        if !self.enabled {
            return Ok(true); // Allow if AuthZEN is disabled
        }

        let authorizer = self.authorizer.as_ref().unwrap();

        let subject = agent_to_authzen_subject(agent);
        let action = AuthZenAction {
            name: "route".to_string(),
            properties: None,
        };
        let resource = agent_type_to_authzen_resource(target_type);
        let context = connection_id.map(|conn_id| {
            json!({
                "connection_id": conn_id,
                "timestamp": chrono::Utc::now().to_rfc3339()
            })
        });

        match authorizer.evaluate(subject, action, resource, context).await {
            Ok(decision) => {
                debug!(
                    agent = %agent,
                    target = %target_type,
                    connection_id = ?connection_id,
                    decision = %decision,
                    "AuthZEN route authorization"
                );
                Ok(decision)
            }
            Err(AuthError::FallbackAllow) => {
                warn!(
                    agent = %agent,
                    target = %target_type,
                    "AuthZEN unavailable, allowing route creation due to fallback policy"
                );
                Ok(true)
            }
            Err(e) => {
                warn!(
                    agent = %agent,
                    target = %target_type,
                    error = %e,
                    "AuthZEN authorization failed"
                );
                Ok(false)
            }
        }
    }

    /// Authorize message publishing
    pub async fn authorize_publish(
        &self,
        source: &Agent,
        target_type: &AgentType,
        target_id: Option<u64>,
        message_size: Option<usize>,
    ) -> Result<bool, ServiceError> {
        if !self.enabled {
            return Ok(true); // Allow if AuthZEN is disabled
        }

        let authorizer = self.authorizer.as_ref().unwrap();

        let subject = agent_to_authzen_subject(source);
        let action = AuthZenAction {
            name: "publish".to_string(),
            properties: message_size.map(|size| json!({"message_size": size})),
        };

        // Create a target agent resource for publishing
        let target_agent = if let Some(agent_id) = target_id {
            Agent::new(target_type.clone(), agent_id)
        } else {
            // Use a default agent ID for agent types without specific ID
            Agent::new(target_type.clone(), 0)
        };

        let resource = agent_to_authzen_resource(&target_agent);
        let context = Some(json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "target_agent_id": target_id
        }));

        match authorizer.evaluate(subject, action, resource, context).await {
            Ok(decision) => {
                debug!(
                    source = %source,
                    target = %target_type,
                    target_id = ?target_id,
                    decision = %decision,
                    "AuthZEN publish authorization"
                );
                Ok(decision)
            }
            Err(AuthError::FallbackAllow) => {
                warn!(
                    source = %source,
                    target = %target_type,
                    "AuthZEN unavailable, allowing publish due to fallback policy"
                );
                Ok(true)
            }
            Err(e) => {
                warn!(
                    source = %source,
                    target = %target_type,
                    error = %e,
                    "AuthZEN publish authorization failed"
                );
                Ok(false)
            }
        }
    }

    /// Authorize subscription
    pub async fn authorize_subscribe(
        &self,
        agent: &Agent,
        target_type: &AgentType,
        target_id: Option<u64>,
    ) -> Result<bool, ServiceError> {
        if !self.enabled {
            return Ok(true); // Allow if AuthZEN is disabled
        }

        let authorizer = self.authorizer.as_ref().unwrap();

        let subject = agent_to_authzen_subject(agent);
        let action = AuthZenAction {
            name: "subscribe".to_string(),
            properties: None,
        };

        // Create a target agent resource for subscription
        let target_agent = if let Some(agent_id) = target_id {
            Agent::new(target_type.clone(), agent_id)
        } else {
            Agent::new(target_type.clone(), 0)
        };

        let resource = agent_to_authzen_resource(&target_agent);
        let context = Some(json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "target_agent_id": target_id
        }));

        match authorizer.evaluate(subject, action, resource, context).await {
            Ok(decision) => {
                debug!(
                    agent = %agent,
                    target = %target_type,
                    target_id = ?target_id,
                    decision = %decision,
                    "AuthZEN subscribe authorization"
                );
                Ok(decision)
            }
            Err(AuthError::FallbackAllow) => {
                warn!(
                    agent = %agent,
                    target = %target_type,
                    "AuthZEN unavailable, allowing subscribe due to fallback policy"
                );
                Ok(true)
            }
            Err(e) => {
                warn!(
                    agent = %agent,
                    target = %target_type,
                    error = %e,
                    "AuthZEN subscribe authorization failed"
                );
                Ok(false)
            }
        }
    }

    /// Clear the authorization cache
    pub async fn clear_cache(&self) {
        if let Some(authorizer) = &self.authorizer {
            authorizer.clear_cache().await;
        }
    }
}

/// Configuration for AuthZEN service integration
#[derive(Debug, Clone)]
pub struct AuthZenServiceConfig {
    pub enabled: bool,
    pub pdp_endpoint: String,
    pub timeout: Duration,
    pub cache_ttl: Duration,
    pub fallback_allow: bool,
    pub max_retries: u32,
}

impl Default for AuthZenServiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            pdp_endpoint: "http://localhost:8080".to_string(),
            timeout: Duration::from_secs(5),
            cache_ttl: Duration::from_secs(300),
            fallback_allow: false,
            max_retries: 3,
        }
    }
}

impl From<ServiceError> for AuthError {
    fn from(err: ServiceError) -> Self {
        AuthError::AuthorizationError(err.to_string())
    }
} 