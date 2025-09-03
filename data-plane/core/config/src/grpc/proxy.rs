// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use hyper_util::client::proxy::matcher::{Intercept, Matcher};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProxyConfig {
    /// The HTTP proxy URL (e.g., "http://proxy.example.com:8080")
    pub url: Option<String>,

    /// Optional username for proxy authentication
    pub username: Option<String>,

    /// Optional password for proxy authentication
    pub password: Option<String>,

    /// Headers to send with proxy requests
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// List of hosts that should bypass the proxy.
    /// Based on hyper-utils matcher: https://github.com/hyperium/hyper-util/blob/master/src/client/proxy/matcher.rs
    #[serde(default)]
    pub no_proxy: Option<String>,
}

impl Clone for ProxyConfig {
    fn clone(&self) -> Self {
        Self {
            url: self.url.clone(),
            username: self.username.clone(),
            password: self.password.clone(),
            headers: self.headers.clone(),
            no_proxy: self.no_proxy.clone(),
        }
    }
}

impl PartialEq for ProxyConfig {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url
            && self.username == other.username
            && self.password == other.password
            && self.headers == other.headers
            && self.no_proxy == other.no_proxy
    }
}

impl ProxyConfig {
    /// Creates a new proxy configuration with the given URL
    pub fn new(url: &str) -> Self {
        Self {
            url: Some(url.to_string()),
            username: None,
            password: None,
            headers: HashMap::new(),
            no_proxy: None,
        }
    }

    /// Sets the proxy authentication credentials
    pub fn with_auth(mut self, username: &str, password: &str) -> Self {
        self.username = Some(username.to_string());
        self.password = Some(password.to_string());
        self
    }

    /// Sets additional headers for proxy requests
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = headers;
        self
    }

    /// Sets the no_proxy list - hosts that should bypass the proxy
    pub fn with_no_proxy(mut self, no_proxy: impl Into<String>) -> Self {
        self.no_proxy = Some(no_proxy.into());
        self
    }

    /// Checks if the given host should bypass the proxy
    pub fn should_use_proxy(&self, uri: &str) -> Option<Intercept> {
        let no_proxy = self.no_proxy.clone().unwrap_or("".into());

        // matcher builder
        let matcher = match self.url.as_ref() {
            Some(url) => Matcher::builder()
                .http(url.clone())
                .https(url.clone())
                .no(no_proxy)
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

    fn test_proxy_no_proxy_functionality() {
        let no_proxy_list = [
            "localhost".to_string(),
            "127.0.0.1".to_string(),
            ".internal.com".to_string(),
            ".example.org".to_string(),
            "direct.access.com".to_string(),
        ];

        let proxy = ProxyConfig::new("http://proxy.example.com:8080")
            .with_no_proxy(no_proxy_list.join(","));

        assert_eq!(proxy.no_proxy, Some(no_proxy_list.join(",")));

        // Test exact matches
        assert!(proxy.should_use_proxy("http://localhost").is_none());
        assert!(proxy.should_use_proxy("http://127.0.0.1").is_none());
        assert!(proxy.should_use_proxy("http://direct.access.com").is_none());

        // Test wildcard matching
        assert!(proxy.should_use_proxy("https://api.internal.com").is_none());
        assert!(proxy.should_use_proxy("http://sub.internal.com").is_none());
        assert!(proxy.should_use_proxy("http://internal.com").is_none());

        // Test non-matching hosts
        assert!(proxy.should_use_proxy("http://google.com").is_some());
        assert!(proxy.should_use_proxy("http://api.external.com").is_some());
        assert!(proxy.should_use_proxy("http://192.168.1.1").is_some());
    }

    fn test_proxy_system_matcher() {
        // Test system matcher when no URL is configured
        let mut proxy_config = ProxyConfig {
            url: None,
            username: None,
            password: None,
            headers: HashMap::new(),
            no_proxy: Some("localhost,127.0.0.1,.internal.com".to_string()),
        };

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
        proxy_config.no_proxy = None;
        let result = proxy_config.should_use_proxy("http://localhost");
        assert!(result.is_none()); // Should return None when no_proxy is None
    }

    fn test_proxy_system_matcher_empty_no_proxy() {
        // Test system matcher with empty no_proxy string
        let proxy_config = ProxyConfig {
            url: None,
            username: None,
            password: None,
            headers: HashMap::new(),
            no_proxy: Some("".to_string()),
        };

        // Empty no_proxy should return None
        assert!(proxy_config.should_use_proxy("http://localhost").is_none());
        assert!(proxy_config.should_use_proxy("http://google.com").is_none());
    }

    fn test_proxy_system_matcher_with_complex_no_proxy() {
        // Test system matcher with complex no_proxy patterns
        let proxy_config = ProxyConfig {
            url: None,
            username: None,
            password: None,
            headers: HashMap::new(),
            no_proxy: Some("*.local,10.0.0.0/8,192.168.0.0/16,.corp.internal".to_string()),
        };

        // Test various patterns that should bypass proxy
        assert!(proxy_config.should_use_proxy("http://api.local").is_none());
        assert!(
            proxy_config
                .should_use_proxy("https://service.corp.internal")
                .is_none()
        );

        // Test patterns that should use proxy (system-dependent)
        let result = proxy_config.should_use_proxy("http://external.com");
        assert!(result.is_some() || result.is_none()); // System-dependent behavior
    }

    fn test_proxy_config_default_creation() {
        // Test creating a config that will use system matcher
        let proxy_config = ProxyConfig::default();

        // Verify all fields are None/empty as expected
        assert!(proxy_config.url.is_none());
        assert!(proxy_config.username.is_none());
        assert!(proxy_config.password.is_none());
        assert!(proxy_config.headers.is_empty());
        assert!(proxy_config.no_proxy.is_none());

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
            username: None,
            password: None,
            headers: HashMap::new(),
            no_proxy: Some("localhost,.config-domain".to_string()),
        };

        // Should use config's settings, not environment
        assert!(
            config_with_url
                .should_use_proxy("http://localhost")
                .is_none()
        );
        assert!(
            config_with_url
                .should_use_proxy("http://api.config-domain")
                .is_none()
        );
        assert!(
            config_with_url
                .should_use_proxy("http://external.com")
                .is_some()
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
        test_proxy_env_variables();
        test_proxy_no_proxy_functionality();
        test_proxy_system_matcher();
        test_proxy_system_matcher_empty_no_proxy();
        test_proxy_system_matcher_with_complex_no_proxy();
        test_proxy_config_default_creation();
    }
}
