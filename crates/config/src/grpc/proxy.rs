// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use hyper_util::client::proxy::matcher::{Intercept, Matcher};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::tls::client::TlsClientConfig as TLSSetting;

#[derive(Clone, Default, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProxyConfig {
    /// The HTTP proxy URL (e.g., "http://proxy.example.com:8080")
    /// If empty, the system proxy settings will be used.
    pub url: Option<String>,

    /// TLS client configuration.
    #[serde(default, rename = "tls")]
    pub tls_setting: TLSSetting,

    /// Optional username for proxy authentication
    pub username: Option<String>,

    /// Optional password for proxy authentication
    pub password: Option<String>,

    /// Headers to send with proxy requests
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

impl ProxyConfig {
    /// Creates a new proxy configuration with the given URL
    pub fn new(url: &str) -> Self {
        Self {
            url: Some(url.to_string()),
            username: None,
            password: None,
            headers: HashMap::new(),
            tls_setting: TLSSetting::default(),
        }
    }

    /// Sets the proxy authentication credentials
    pub fn with_auth(self, username: &str, password: &str) -> Self {
        Self {
            username: Some(username.to_string()),
            password: Some(password.to_string()),
            ..self
        }
    }

    /// Sets additional headers for proxy requests
    pub fn with_headers(self, headers: HashMap<String, String>) -> Self {
        Self { headers, ..self }
    }

    /// Checks if the given host should bypass the proxy
    pub fn should_use_proxy(&self, uri: impl Into<String>) -> Option<Intercept> {
        let uri = uri.into();

        // matcher builder
        let matcher = match self.url.as_ref() {
            Some(url) => Matcher::builder()
                .http(url.clone())
                .https(url.clone())
                .build(),
            None => Matcher::from_system(),
        };

        // Convert string uri into http::Uri
        let dst = uri.parse::<http::Uri>().unwrap();

        // Check if this should bypass the proxy
        matcher.intercept(&dst)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn test_proxy_config() {
        let proxy = ProxyConfig::new("http://proxy.example.com:8080");
        assert_eq!(proxy.url, Some("http://proxy.example.com:8080".to_string()));
        assert_eq!(proxy.username, None);
        assert_eq!(proxy.password, None);
        assert!(proxy.headers.is_empty());

        let proxy_with_auth = proxy.with_auth("user", "pass");
        assert_eq!(proxy_with_auth.username, Some("user".to_string()));
        assert_eq!(proxy_with_auth.password, Some("pass".to_string()));

        let mut headers = HashMap::new();
        headers.insert("X-Custom".to_string(), "value".to_string());
        let proxy_with_headers =
            ProxyConfig::new("http://proxy.example.com:8080").with_headers(headers.clone());
        assert_eq!(proxy_with_headers.headers, headers);
    }

    fn test_proxy_match() {
        let proxy = ProxyConfig::new("http://proxy.example.com:8080");
        assert!(proxy.should_use_proxy("http://example.com").is_some());
    }

    fn test_proxy_system_matcher() {
        // Test system matcher when no URL is configured
        let proxy_config = ProxyConfig::default();

        // System matcher should still respect no_proxy settings
        assert!(proxy_config.should_use_proxy("http://localhost").is_none());
        assert!(proxy_config.should_use_proxy("http://127.0.0.1").is_none());
        assert!(
            proxy_config
                .should_use_proxy("https://api.internal.com")
                .is_none()
        );

        // Test with external hosts - behavior depends on system proxy settings
        // We can't assert specific behavior since it depends on the system configuration,
        // but we can ensure the function doesn't panic and returns a valid result
        let result = proxy_config.should_use_proxy("http://google.com");
        assert!(result.is_some() || result.is_none()); // Either is valid

        // Test system matcher with no no_proxy settings
        let result = proxy_config.should_use_proxy("http://localhost");
        assert!(result.is_none()); // Should return None when no_proxy is None
    }

    fn test_proxy_config_default_creation() {
        // Test creating a config that will use system matcher
        let proxy_config = ProxyConfig::default();

        // Verify all fields are None/empty as expected
        assert!(proxy_config.url.is_none());
        assert!(proxy_config.username.is_none());
        assert!(proxy_config.password.is_none());
        assert!(proxy_config.headers.is_empty());

        // Any call should return None when no_proxy is None
        assert!(
            proxy_config
                .should_use_proxy("http://any-host.com")
                .is_none()
        );
    }

    #[allow(clippy::disallowed_methods)]
    fn test_proxy_env_variables() {
        // Test how different environment variables work with system matcher

        // Save original environment variables
        let original_env = [
            ("https_proxy", std::env::var("https_proxy").ok()),
            ("HTTPS_PROXY", std::env::var("HTTPS_PROXY").ok()),
            ("http_proxy", std::env::var("http_proxy").ok()),
            ("HTTP_PROXY", std::env::var("HTTP_PROXY").ok()),
            ("no_proxy", std::env::var("no_proxy").ok()),
            ("NO_PROXY", std::env::var("NO_PROXY").ok()),
            ("all_proxy", std::env::var("all_proxy").ok()),
            ("ALL_PROXY", std::env::var("ALL_PROXY").ok()),
        ];

        // Clean up environment first
        for (key, _) in &original_env {
            unsafe {
                std::env::remove_var(key);
            }
        }

        // Test 1: Config with URL should ignore environment completely
        let config_with_url = ProxyConfig {
            url: Some("http://config-proxy.example.com:8080".to_string()),
            tls_setting: TLSSetting::default(),
            username: None,
            password: None,
            headers: HashMap::new(),
        };

        // Should use config's settings, not environment
        assert!(
            config_with_url
                .should_use_proxy("http://localhost")
                .is_some_and(|proxy| { *proxy.uri() == "http://config-proxy.example.com:8080/" })
        );

        // Test 2: Config without URL
        let config_without_url = ProxyConfig::default();

        // Now set environment variables and test system matcher behavior
        unsafe {
            std::env::set_var("http_proxy", "http://env-proxy.example.com:8080");
            std::env::set_var("no_proxy", "localhost,.env-domain");
        }

        // Let's see if no_proxy is respected
        assert!(
            config_without_url
                .should_use_proxy("http://localhost")
                .is_none()
        );
        assert!(
            config_without_url
                .should_use_proxy("http://api.system-bypass")
                .is_some()
        );

        // Unset no_proxy
        unsafe {
            std::env::remove_var("no_proxy");
        }

        // When no_proxy is None, it should always use the proxy
        assert!(
            config_without_url
                .should_use_proxy("http://localhost")
                .is_some()
        );

        // Restore original environment variables
        for (key, original_value) in original_env {
            unsafe {
                match original_value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn run_all_tests() {
        // Run tests consecutively as we are using the set/unset env variables,
        // which may influence concurrent test execution
        test_proxy_config();
        test_proxy_match();
        test_proxy_env_variables();
        test_proxy_system_matcher();
        test_proxy_config_default_creation();
    }
}
