package config

import (
	"fmt"
	"os"
	"path/filepath"
)

// Environment variable names for CLI flag overrides
const (
	EnvSlimEndpoint = "SLIM_ENDPOINT"
	EnvSlimInsecure = "SLIM_TLS_INSECURE"
	EnvSlimTLSCert  = "SLIM_TLS_CERT"
	EnvSlimTLSKey   = "SLIM_TLS_KEY"
	EnvRustLog      = "RUST_LOG" // Standard Rust logging env var
)

// ConfigManager handles SLIM configuration for bindings initialization
type ConfigManager struct {
	configPath string
	tempConfig bool // Whether using a temporary config file
}

// NewConfigManager creates a new config manager
func NewConfigManager(configPath string) *ConfigManager {
	return &ConfigManager{
		configPath: configPath,
		tempConfig: false,
	}
}

// GetConfigPath returns the configuration file path to use with bindings.
// If no config is provided, creates a minimal temporary config that uses environment variables.
// No validation is performed - the bindings library handles all validation.
func (cm *ConfigManager) GetConfigPath() (string, error) {
	if cm.configPath != "" {
		// Return user-provided config as-is
		// No existence check - let bindings handle errors
		return cm.configPath, nil
	}

	// Create minimal temporary config with env var references
	return cm.createDefaultTempConfig()
}

// createDefaultTempConfig creates a minimal default configuration with environment variable references
func (cm *ConfigManager) createDefaultTempConfig() (string, error) {
	tempDir := os.TempDir()
	tempFile := filepath.Join(tempDir, fmt.Sprintf("slimctl-config-%d.yaml", os.Getpid()))

	// Minimal default configuration that delegates to environment variables
	// The bindings will substitute ${env:VAR} references automatically
	configTemplate := `# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
# Auto-generated temporary configuration for slimctl

runtime:
  n_cores: 0
  thread_name: "slim-worker"
  drain_timeout: 10s

tracing:
  log_level: info
  display_thread_names: true
  display_thread_ids: true

services:
  slim/0:
    dataplane:
      servers:
        - endpoint: "${env:SLIM_ENDPOINT}"
          tls:
            insecure: true
      clients: []
`

	if err := os.WriteFile(tempFile, []byte(configTemplate), 0600); err != nil {
		return "", fmt.Errorf("failed to create temporary config: %w", err)
	}

	cm.tempConfig = true
	cm.configPath = tempFile
	return tempFile, nil
}

// Cleanup removes temporary configuration file if created
func (cm *ConfigManager) Cleanup() error {
	if cm.tempConfig && cm.configPath != "" {
		return os.Remove(cm.configPath)
	}
	return nil
}

// SetEnvironmentOverrides sets environment variables for CLI flag overrides.
// These will be substituted by the bindings when processing ${env:VAR} references.
func SetEnvironmentOverrides(endpoint, tlsCert, tlsKey *string, insecure *bool) error {
	// Set endpoint override
	if endpoint != nil && *endpoint != "" {
		if err := os.Setenv(EnvSlimEndpoint, *endpoint); err != nil {
			return fmt.Errorf("failed to set endpoint override: %w", err)
		}
	} else {
		// Set default if not specified
		if err := os.Setenv(EnvSlimEndpoint, "127.0.0.1:8080"); err != nil {
			return err
		}
	}

	// Set insecure mode override
	if insecure != nil {
		insecureStr := "false"
		if *insecure {
			insecureStr = "true"
		}
		if err := os.Setenv(EnvSlimInsecure, insecureStr); err != nil {
			return fmt.Errorf("failed to set insecure override: %w", err)
		}
	}

	// Set TLS certificate override
	if tlsCert != nil && *tlsCert != "" {
		if err := os.Setenv(EnvSlimTLSCert, *tlsCert); err != nil {
			return fmt.Errorf("failed to set TLS cert override: %w", err)
		}
	}

	// Set TLS key override
	if tlsKey != nil && *tlsKey != "" {
		if err := os.Setenv(EnvSlimTLSKey, *tlsKey); err != nil {
			return fmt.Errorf("failed to set TLS key override: %w", err)
		}
	}

	return nil
}

// GetDisplayEndpoint returns the endpoint that will be used for display purposes
func GetDisplayEndpoint() string {
	endpoint := os.Getenv(EnvSlimEndpoint)
	if endpoint == "" {
		endpoint = "127.0.0.1:8080" // Default
	}
	return endpoint
}

// GetDisplayLogLevel returns the log level that will be used
func GetDisplayLogLevel() string {
	logLevel := os.Getenv(EnvRustLog)
	if logLevel == "" {
		logLevel = "info"
	}
	return logLevel
}

// IsTLSEnabled returns true if TLS is configured (not insecure and cert provided)
func IsTLSEnabled() bool {
	insecure := os.Getenv(EnvSlimInsecure)
	cert := os.Getenv(EnvSlimTLSCert)

	// TLS is enabled if we're not in insecure mode and have a cert
	return insecure != "true" && cert != ""
}
