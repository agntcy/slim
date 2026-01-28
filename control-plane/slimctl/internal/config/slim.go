package config

import (
	"fmt"
	"os"

	"gopkg.in/yaml.v3"

	slim "github.com/agntcy/slim-bindings-go"
)

// SlimConfig represents the configuration for a SLIM instance.
type SlimConfig struct {
	Endpoint string    `yaml:"endpoint"`
	TLS      TLSConfig `yaml:"tls"`
	// Room for future expansion: Auth, HTTP2, Keepalive, etc.
}

// TLSConfig represents TLS configuration options.
type TLSConfig struct {
	Insecure bool   `yaml:"insecure"`
	CertFile string `yaml:"cert_file"`
	KeyFile  string `yaml:"key_file"`
	// Future: ClientCA, TLSVersion, etc.
}

// DefaultSlimConfig returns a SlimConfig with default values.
func DefaultSlimConfig() *SlimConfig {
	return &SlimConfig{
		Endpoint: "127.0.0.1:8080",
		TLS: TLSConfig{
			Insecure: true, // Default to insecure for backward compatibility
		},
	}
}

// LoadFromFile loads configuration from a YAML file.
func LoadFromFile(path string) (*SlimConfig, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("failed to read config file: %w", err)
	}

	config := DefaultSlimConfig()
	if err := yaml.Unmarshal(data, config); err != nil {
		return nil, fmt.Errorf("failed to parse config file: %w", err)
	}

	return config, nil
}

// Validate checks if the configuration is valid.
func (c *SlimConfig) Validate() error {
	if c.Endpoint == "" {
		return fmt.Errorf("endpoint cannot be empty")
	}

	// If TLS is enabled, both cert and key must be provided
	if !c.TLS.Insecure {
		if c.TLS.CertFile == "" {
			return fmt.Errorf("tls.cert_file is required when TLS is enabled")
		}
		if c.TLS.KeyFile == "" {
			return fmt.Errorf("tls.key_file is required when TLS is enabled")
		}

		// Check if certificate files exist
		if _, err := os.Stat(c.TLS.CertFile); os.IsNotExist(err) {
			return fmt.Errorf("certificate file not found: %s", c.TLS.CertFile)
		}
		if _, err := os.Stat(c.TLS.KeyFile); os.IsNotExist(err) {
			return fmt.Errorf("key file not found: %s", c.TLS.KeyFile)
		}
	}

	return nil
}

// ToServerConfig converts SlimConfig to the golang bindings ServerConfig.
func (c *SlimConfig) ToServerConfig() (slim.ServerConfig, error) {
	if err := c.Validate(); err != nil {
		return slim.ServerConfig{}, err
	}

	if c.TLS.Insecure {
		return slim.NewInsecureServerConfig(c.Endpoint), nil
	}

	// Create a secure server config with TLS
	// Use TlsSourceFile to let the bindings handle file loading and auto-reload
	config := slim.NewServerConfig(c.Endpoint)
	config.Tls.Insecure = false
	config.Tls.Source = slim.TlsSourceFile{
		Cert: c.TLS.CertFile,
		Key:  c.TLS.KeyFile,
	}

	return config, nil
}

// MergeFlags merges command-line flag overrides into the configuration.
func (c *SlimConfig) MergeFlags(endpoint *string, insecure *bool, certFile *string, keyFile *string) {
	if endpoint != nil && *endpoint != "" {
		c.Endpoint = *endpoint
	}

	if insecure != nil {
		c.TLS.Insecure = *insecure
	}

	if certFile != nil && *certFile != "" {
		c.TLS.CertFile = *certFile
	}

	if keyFile != nil && *keyFile != "" {
		c.TLS.KeyFile = *keyFile
	}
}
