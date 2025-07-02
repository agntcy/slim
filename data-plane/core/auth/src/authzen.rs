// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! AuthZEN authorization client implementation
//! 
//! This module provides integration with OpenID AuthZEN standard authorization APIs.
//! See: https://openid.github.io/authzen/

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::errors::AuthError;

/// AuthZEN Subject representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthZenSubject {
    #[serde(rename = "type")]
    pub subject_type: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<serde_json::Value>,
}

/// AuthZEN Action representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthZenAction {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<serde_json::Value>,
}

/// AuthZEN Resource representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthZenResource {
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<serde_json::Value>,
}

/// AuthZEN evaluation request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthZenRequest {
    pub subject: AuthZenSubject,
    pub action: AuthZenAction,
    pub resource: AuthZenResource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}

/// AuthZEN evaluation response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthZenResponse {
    pub decision: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}

/// AuthZEN batch evaluations request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthZenEvaluationsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<AuthZenSubject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<AuthZenAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<AuthZenResource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
    pub evaluations: Vec<AuthZenRequest>,
}

/// AuthZEN batch evaluations response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthZenEvaluationsResponse {
    pub evaluations: Vec<AuthZenResponse>,
}

/// Cache entry for authorization decisions
#[derive(Debug, Clone)]
struct CacheEntry {
    decision: bool,
    expires_at: Instant,
}

/// AuthZEN cache key
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct CacheKey {
    subject: String,
    action: String,
    resource: String,
    context_hash: u64,
}

/// AuthZEN configuration
#[derive(Debug, Clone)]
pub struct AuthZenConfig {
    pub pdp_endpoint: String,
    pub timeout: Duration,
    pub cache_ttl: Duration,
    pub fallback_allow: bool,
    pub max_retries: u32,
}

impl Default for AuthZenConfig {
    fn default() -> Self {
        Self {
            pdp_endpoint: "http://localhost:8080".to_string(),
            timeout: Duration::from_secs(5),
            cache_ttl: Duration::from_secs(300), // 5 minutes
            fallback_allow: false, // fail-closed by default
            max_retries: 3,
        }
    }
}

/// AuthZEN authorization client
pub struct AuthZenAuthorizer {
    client: Client,
    config: AuthZenConfig,
    cache: Arc<RwLock<HashMap<CacheKey, CacheEntry>>>,
}

impl AuthZenAuthorizer {
    /// Create a new AuthZEN authorizer
    pub fn new(config: AuthZenConfig) -> Result<Self, AuthError> {
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| AuthError::ConfigurationError(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client,
            config,
            cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Evaluate a single authorization request
    pub async fn evaluate(
        &self,
        subject: AuthZenSubject,
        action: AuthZenAction,
        resource: AuthZenResource,
        context: Option<serde_json::Value>,
    ) -> Result<bool, AuthError> {
        // Check cache first
        let cache_key = self.create_cache_key(&subject, &action, &resource, &context);
        if let Some(cached) = self.get_cached_decision(&cache_key).await {
            return Ok(cached);
        }

        let request = AuthZenRequest {
            subject: subject.clone(),
            action: action.clone(),
            resource: resource.clone(),
            context: context.clone(),
        };

        match self.make_request(&request).await {
            Ok(decision) => {
                // Cache the decision
                self.cache_decision(cache_key, decision).await;
                Ok(decision)
            }
            Err(AuthError::FallbackAllow) => {
                // Return true for fallback allow and cache it
                self.cache_decision(cache_key, true).await;
                Ok(true)
            }
            Err(e) => Err(e),
        }
    }

    /// Evaluate multiple authorization requests in a batch
    pub async fn evaluate_batch(
        &self,
        requests: Vec<AuthZenRequest>,
    ) -> Result<Vec<bool>, AuthError> {
        if requests.is_empty() {
            return Ok(vec![]);
        }

        // For now, use the single evaluation endpoint multiple times
        // TODO: Implement proper batch evaluation using /access/v1/evaluations
        let mut results = Vec::with_capacity(requests.len());
        
        for request in requests {
            let decision = self.make_request(&request).await?;
            results.push(decision);
        }
        
        Ok(results)
    }

    /// Make HTTP request to AuthZEN PDP
    async fn make_request(&self, request: &AuthZenRequest) -> Result<bool, AuthError> {
        let url = format!("{}/access/v1/evaluation", self.config.pdp_endpoint);
        
        let response = self.client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(request)
            .send()
            .await
            .map_err(|e| {
                tracing::error!("AuthZEN request failed: {}", e);
                if self.config.fallback_allow {
                    tracing::warn!("Falling back to ALLOW due to AuthZEN unavailability");
                    return AuthError::FallbackAllow;
                }
                AuthError::NetworkError(format!("AuthZEN request failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            tracing::error!("AuthZEN returned error status {}: {}", status, body);
            
            if self.config.fallback_allow {
                tracing::warn!("Falling back to ALLOW due to AuthZEN error");
                return Ok(true);
            }
            
            return Err(AuthError::AuthorizationError(
                format!("AuthZEN returned error status {}: {}", status, body)
            ));
        }

        let authzen_response: AuthZenResponse = response
            .json()
            .await
            .map_err(|e| AuthError::ParseError(format!("Invalid AuthZEN response: {}", e)))?;

        tracing::debug!(
            subject = %request.subject.id,
            action = %request.action.name, 
            resource = %request.resource.id,
            decision = %authzen_response.decision,
            "AuthZEN decision"
        );

        Ok(authzen_response.decision)
    }

    /// Create cache key for the request
    fn create_cache_key(
        &self,
        subject: &AuthZenSubject,
        action: &AuthZenAction, 
        resource: &AuthZenResource,
        context: &Option<serde_json::Value>,
    ) -> CacheKey {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let context_hash = {
            let mut hasher = DefaultHasher::new();
            context.hash(&mut hasher);
            hasher.finish()
        };

        CacheKey {
            subject: format!("{}:{}", subject.subject_type, subject.id),
            action: action.name.clone(),
            resource: format!("{}:{}", resource.resource_type, resource.id),
            context_hash,
        }
    }

    /// Get cached decision if available and not expired
    async fn get_cached_decision(&self, key: &CacheKey) -> Option<bool> {
        let cache = self.cache.read().await;
        
        if let Some(entry) = cache.get(key) {
            if entry.expires_at > Instant::now() {
                tracing::debug!("Cache hit for authorization decision");
                return Some(entry.decision);
            }
        }
        
        None
    }

    /// Cache authorization decision
    async fn cache_decision(&self, key: CacheKey, decision: bool) {
        let entry = CacheEntry {
            decision,
            expires_at: Instant::now() + self.config.cache_ttl,
        };
        
        let mut cache = self.cache.write().await;
        cache.insert(key, entry);
        
        // Simple cleanup: remove expired entries occasionally
        if cache.len() > 1000 {
            cache.retain(|_, entry| entry.expires_at > Instant::now());
        }
    }

    /// Clear authorization cache
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
        tracing::info!("AuthZEN cache cleared");
    }
}

/// Helper trait for converting SLIM entities to AuthZEN format
pub trait ToAuthZen {
    fn to_authzen_subject(&self) -> AuthZenSubject;
    fn to_authzen_resource(&self) -> AuthZenResource;
}

// We'll implement this trait for SLIM types in the service integration 

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json_string, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_authzen_config_default() {
        let config = AuthZenConfig::default();
        assert_eq!(config.pdp_endpoint, "http://localhost:8080");
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert_eq!(config.cache_ttl, Duration::from_secs(300));
        assert_eq!(config.fallback_allow, false);
        assert_eq!(config.max_retries, 3);
    }

    #[tokio::test]
    async fn test_authzen_authorizer_creation() {
        let config = AuthZenConfig::default();
        let authorizer = AuthZenAuthorizer::new(config);
        assert!(authorizer.is_ok());
    }

    #[tokio::test]
    async fn test_authzen_evaluation_success() {
        // Start a mock server
        let mock_server = MockServer::start().await;

        // Mock the AuthZEN evaluation endpoint
        Mock::given(method("POST"))
            .and(path("/access/v1/evaluation"))
            .respond_with(ResponseTemplate::new(200).set_body_json(AuthZenResponse {
                decision: true,
                context: None,
            }))
            .mount(&mock_server)
            .await;

        // Create config pointing to mock server
        let config = AuthZenConfig {
            pdp_endpoint: mock_server.uri(),
            timeout: Duration::from_secs(5),
            cache_ttl: Duration::from_secs(300),
            fallback_allow: false,
            max_retries: 3,
        };

        let authorizer = AuthZenAuthorizer::new(config).unwrap();

        let subject = AuthZenSubject {
            subject_type: "agent".to_string(),
            id: "test-agent".to_string(),
            properties: None,
        };

        let action = AuthZenAction {
            name: "publish".to_string(),
            properties: None,
        };

        let resource = AuthZenResource {
            resource_type: "route".to_string(),
            id: "test-route".to_string(),
            properties: None,
        };

        let decision = authorizer.evaluate(subject, action, resource, None).await;
        assert!(decision.is_ok());
        assert_eq!(decision.unwrap(), true);
    }

    #[tokio::test]
    async fn test_authzen_evaluation_denied() {
        // Start a mock server
        let mock_server = MockServer::start().await;

        // Mock the AuthZEN evaluation endpoint returning false
        Mock::given(method("POST"))
            .and(path("/access/v1/evaluation"))
            .respond_with(ResponseTemplate::new(200).set_body_json(AuthZenResponse {
                decision: false,
                context: None,
            }))
            .mount(&mock_server)
            .await;

        let config = AuthZenConfig {
            pdp_endpoint: mock_server.uri(),
            timeout: Duration::from_secs(5),
            cache_ttl: Duration::from_secs(300),
            fallback_allow: false,
            max_retries: 3,
        };

        let authorizer = AuthZenAuthorizer::new(config).unwrap();

        let subject = AuthZenSubject {
            subject_type: "agent".to_string(),
            id: "test-agent".to_string(),
            properties: None,
        };

        let action = AuthZenAction {
            name: "publish".to_string(),
            properties: None,
        };

        let resource = AuthZenResource {
            resource_type: "route".to_string(),
            id: "test-route".to_string(),
            properties: None,
        };

        let decision = authorizer.evaluate(subject, action, resource, None).await;
        assert!(decision.is_ok());
        assert_eq!(decision.unwrap(), false);
    }

    #[tokio::test]
    async fn test_authzen_fallback_allow() {
        // Create config with fallback_allow = true and invalid endpoint
        let config = AuthZenConfig {
            pdp_endpoint: "http://invalid-endpoint:9999".to_string(),
            timeout: Duration::from_millis(100),
            cache_ttl: Duration::from_secs(300),
            fallback_allow: true,
            max_retries: 1,
        };

        let authorizer = AuthZenAuthorizer::new(config).unwrap();

        let subject = AuthZenSubject {
            subject_type: "agent".to_string(),
            id: "test-agent".to_string(),
            properties: None,
        };

        let action = AuthZenAction {
            name: "publish".to_string(),
            properties: None,
        };

        let resource = AuthZenResource {
            resource_type: "route".to_string(),
            id: "test-route".to_string(),
            properties: None,
        };

        let decision = authorizer.evaluate(subject, action, resource, None).await;
        assert!(decision.is_ok());
        assert_eq!(decision.unwrap(), true); // Should fallback to allow
    }

    #[tokio::test]
    async fn test_authzen_cache() {
        let mock_server = MockServer::start().await;

        // Mock the evaluation endpoint to return true
        Mock::given(method("POST"))
            .and(path("/access/v1/evaluation"))
            .respond_with(ResponseTemplate::new(200).set_body_json(AuthZenResponse {
                decision: true,
                context: None,
            }))
            .expect(1) // Should only be called once due to caching
            .mount(&mock_server)
            .await;

        let config = AuthZenConfig {
            pdp_endpoint: mock_server.uri(),
            timeout: Duration::from_secs(5),
            cache_ttl: Duration::from_secs(300),
            fallback_allow: false,
            max_retries: 3,
        };

        let authorizer = AuthZenAuthorizer::new(config).unwrap();

        let subject = AuthZenSubject {
            subject_type: "agent".to_string(),
            id: "test-agent".to_string(),
            properties: None,
        };

        let action = AuthZenAction {
            name: "publish".to_string(),
            properties: None,
        };

        let resource = AuthZenResource {
            resource_type: "route".to_string(),
            id: "test-route".to_string(),
            properties: None,
        };

        // First call - should hit the server
        let decision1 = authorizer.evaluate(subject.clone(), action.clone(), resource.clone(), None).await;
        assert!(decision1.is_ok());
        assert_eq!(decision1.unwrap(), true);

        // Second call - should use cache
        let decision2 = authorizer.evaluate(subject, action, resource, None).await;
        assert!(decision2.is_ok());
        assert_eq!(decision2.unwrap(), true);
    }
} 