package config

import (
	"fmt"
	"os"
	"time"

	"gopkg.in/yaml.v3"

	slim "github.com/agntcy/slim-bindings-go"
)

// FullConfig represents the complete SLIM configuration
type FullConfig struct {
	Runtime  RuntimeConfig            `yaml:"runtime"`
	Tracing  TracingConfig            `yaml:"tracing"`
	Services map[string]ServiceConfig `yaml:"services"`
}

// RuntimeConfig represents the runtime configuration
type RuntimeConfig struct {
	NCores       int    `yaml:"n_cores"`
	ThreadName   string `yaml:"thread_name"`
	DrainTimeout string `yaml:"drain_timeout"`
}

// TracingConfig represents the tracing/logging configuration
type TracingConfig struct {
	LogLevel           string `yaml:"log_level"`
	DisplayThreadNames bool   `yaml:"display_thread_names"`
	DisplayThreadIds   bool   `yaml:"display_thread_ids"`
}

// ServiceConfig represents a single service configuration
type ServiceConfig struct {
	Dataplane DataplaneConfig `yaml:"dataplane"`
}

// DataplaneConfig represents the dataplane configuration
type DataplaneConfig struct {
	Servers []ServerConfig `yaml:"servers"`
	Clients []interface{}  `yaml:"clients"`
}

// ServerConfig represents a server configuration
type ServerConfig struct {
	Endpoint             string           `yaml:"endpoint"`
	TLS                  TLSServerConfig  `yaml:"tls,omitempty"`
	HTTP2Only            *bool            `yaml:"http2_only,omitempty"`
	MaxFrameSize         *uint32          `yaml:"max_frame_size,omitempty"`
	MaxConcurrentStreams *uint32          `yaml:"max_concurrent_streams,omitempty"`
	Keepalive            *KeepaliveConfig `yaml:"keepalive,omitempty"`
	Auth                 *AuthConfig      `yaml:"auth,omitempty"`
}

// TLSServerConfig represents TLS server configuration
type TLSServerConfig struct {
	Insecure bool             `yaml:"insecure,omitempty"`
	Source   *TLSSourceConfig `yaml:"source,omitempty"`
}

// TLSSourceConfig represents TLS source configuration
type TLSSourceConfig struct {
	Type string `yaml:"type"` // "file"
	Cert string `yaml:"cert"`
	Key  string `yaml:"key"`
}

// KeepaliveConfig represents keepalive configuration
type KeepaliveConfig struct {
	MaxConnectionIdle     string `yaml:"max_connection_idle,omitempty"`
	MaxConnectionAge      string `yaml:"max_connection_age,omitempty"`
	MaxConnectionAgeGrace string `yaml:"max_connection_age_grace,omitempty"`
	Time                  string `yaml:"time,omitempty"`
	Timeout               string `yaml:"timeout,omitempty"`
}

// AuthConfig represents authentication configuration
type AuthConfig struct {
	Type string `yaml:"type"` // "none", "basic", "jwt"
}

// DefaultFullConfig returns a FullConfig with default values matching SLIM production defaults
func DefaultFullConfig() *FullConfig {
	return &FullConfig{
		Runtime: RuntimeConfig{
			NCores:       0, // 0 means use all available cores
			ThreadName:   "slim-worker",
			DrainTimeout: "10s",
		},
		Tracing: TracingConfig{
			LogLevel:           "info",
			DisplayThreadNames: true,
			DisplayThreadIds:   true,
		},
		Services: map[string]ServiceConfig{
			"slim/0": {
				Dataplane: DataplaneConfig{
					Servers: []ServerConfig{
						{
							Endpoint: "127.0.0.1:8080",
							TLS: TLSServerConfig{
								Insecure: true,
							},
						},
					},
					Clients: []interface{}{},
				},
			},
		},
	}
}

// LoadFullConfig loads configuration from a YAML file
func LoadFullConfig(path string) (*FullConfig, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("failed to read config file: %w", err)
	}

	config := DefaultFullConfig()
	if err := yaml.Unmarshal(data, config); err != nil {
		return nil, fmt.Errorf("failed to parse config file: %w", err)
	}

	return config, nil
}

// Validate checks if the configuration is valid
func (c *FullConfig) Validate() error {
	// Check services
	if len(c.Services) == 0 {
		return fmt.Errorf("at least one service is required")
	}

	// Get the first service (slim/0)
	var firstService ServiceConfig
	var found bool
	for _, svc := range c.Services {
		firstService = svc
		found = true
		break
	}
	if !found {
		return fmt.Errorf("no service configuration found")
	}

	// Check servers
	if len(firstService.Dataplane.Servers) == 0 {
		return fmt.Errorf("at least one server is required in service configuration")
	}

	server := firstService.Dataplane.Servers[0]
	if server.Endpoint == "" {
		return fmt.Errorf("server endpoint cannot be empty")
	}

	// If TLS is enabled, validate source
	if !server.TLS.Insecure {
		if server.TLS.Source == nil {
			return fmt.Errorf("tls.source is required when TLS is enabled")
		}
		if server.TLS.Source.Type != "file" {
			return fmt.Errorf("only 'file' TLS source type is supported")
		}
		if server.TLS.Source.Cert == "" {
			return fmt.Errorf("tls.source.cert is required when TLS is enabled")
		}
		if server.TLS.Source.Key == "" {
			return fmt.Errorf("tls.source.key is required when TLS is enabled")
		}

		// Check if certificate files exist
		if _, err := os.Stat(server.TLS.Source.Cert); os.IsNotExist(err) {
			return fmt.Errorf("certificate file not found: %s", server.TLS.Source.Cert)
		}
		if _, err := os.Stat(server.TLS.Source.Key); os.IsNotExist(err) {
			return fmt.Errorf("key file not found: %s", server.TLS.Source.Key)
		}
	}

	return nil
}

// MergeFlags merges command-line flag overrides into the configuration
func (c *FullConfig) MergeFlags(endpoint *string, insecure *bool, certFile *string, keyFile *string) {
	// Get the first service
	var firstService *ServiceConfig
	for k := range c.Services {
		svc := c.Services[k]
		firstService = &svc
		break
	}
	if firstService == nil || len(firstService.Dataplane.Servers) == 0 {
		return
	}

	server := &firstService.Dataplane.Servers[0]

	if endpoint != nil && *endpoint != "" {
		server.Endpoint = *endpoint
	}

	if insecure != nil {
		server.TLS.Insecure = *insecure
	}

	if certFile != nil && *certFile != "" {
		if server.TLS.Source == nil {
			server.TLS.Source = &TLSSourceConfig{Type: "file"}
		}
		server.TLS.Source.Cert = *certFile
		server.TLS.Insecure = false
	}

	if keyFile != nil && *keyFile != "" {
		if server.TLS.Source == nil {
			server.TLS.Source = &TLSSourceConfig{Type: "file"}
		}
		server.TLS.Source.Key = *keyFile
		server.TLS.Insecure = false
	}

	// Update the service back in the map
	for k := range c.Services {
		c.Services[k] = *firstService
		break
	}
}

// ToBindingsConfigs converts FullConfig to bindings configuration types
func (c *FullConfig) ToBindingsConfigs() (slim.RuntimeConfig, slim.TracingConfig, []slim.ServiceConfig) {
	// Convert Runtime config
	runtimeConfig := slim.NewRuntimeConfig()
	runtimeConfig.NCores = uint64(c.Runtime.NCores)
	runtimeConfig.ThreadName = c.Runtime.ThreadName
	// Parse drain timeout string to Duration
	if duration, err := time.ParseDuration(c.Runtime.DrainTimeout); err == nil {
		runtimeConfig.DrainTimeout = duration
	} else {
		// Default to 10 seconds if parsing fails
		runtimeConfig.DrainTimeout = 10 * time.Second
	}

	// Convert Tracing config
	tracingConfig := slim.NewTracingConfig()
	tracingConfig.LogLevel = c.Tracing.LogLevel
	tracingConfig.DisplayThreadNames = c.Tracing.DisplayThreadNames
	tracingConfig.DisplayThreadIds = c.Tracing.DisplayThreadIds
	tracingConfig.Filters = []string{} // Empty filters by default

	// Convert Services
	serviceConfigs := make([]slim.ServiceConfig, 0, len(c.Services))
	for _, svc := range c.Services {
		serviceConfig := slim.NewServiceConfig()
		// Leave node_id and group_name as nil - they're optional
		// The service will use defaults if not set

		// Convert dataplane config
		for _, srv := range svc.Dataplane.Servers {
			serverConfig := c.serverConfigToBindings(srv)
			serviceConfig.Dataplane.Servers = append(serviceConfig.Dataplane.Servers, serverConfig)
		}

		serviceConfigs = append(serviceConfigs, serviceConfig)
	}

	return runtimeConfig, tracingConfig, serviceConfigs
}

// serverConfigToBindings converts a ServerConfig to bindings ServerConfig
func (c *FullConfig) serverConfigToBindings(srv ServerConfig) slim.ServerConfig {
	var config slim.ServerConfig

	if srv.TLS.Insecure {
		config = slim.NewInsecureServerConfig(srv.Endpoint)
	} else {
		config = slim.NewServerConfig(srv.Endpoint)
		config.Tls.Insecure = false
		if srv.TLS.Source != nil {
			config.Tls.Source = slim.TlsSourceFile{
				Cert: srv.TLS.Source.Cert,
				Key:  srv.TLS.Source.Key,
			}
		}
	}

	// Apply optional settings
	if srv.HTTP2Only != nil {
		config.Http2Only = *srv.HTTP2Only
	}
	if srv.MaxFrameSize != nil {
		config.MaxFrameSize = srv.MaxFrameSize
	}
	if srv.MaxConcurrentStreams != nil {
		config.MaxConcurrentStreams = srv.MaxConcurrentStreams
	}

	return config
}

// GetServerConfig extracts the first server configuration as a bindings ServerConfig
func (c *FullConfig) GetServerConfig() (slim.ServerConfig, error) {
	if err := c.Validate(); err != nil {
		return slim.ServerConfig{}, err
	}

	// Get the first service
	for _, svc := range c.Services {
		if len(svc.Dataplane.Servers) > 0 {
			return c.serverConfigToBindings(svc.Dataplane.Servers[0]), nil
		}
	}

	return slim.ServerConfig{}, fmt.Errorf("no server configuration found")
}

// GetEndpoint returns the endpoint of the first server
func (c *FullConfig) GetEndpoint() string {
	for _, svc := range c.Services {
		if len(svc.Dataplane.Servers) > 0 {
			return svc.Dataplane.Servers[0].Endpoint
		}
	}
	return ""
}

// IsTLSEnabled returns true if TLS is enabled for the first server
func (c *FullConfig) IsTLSEnabled() bool {
	for _, svc := range c.Services {
		if len(svc.Dataplane.Servers) > 0 {
			return !svc.Dataplane.Servers[0].TLS.Insecure
		}
	}
	return false
}

// GetTLSCertFile returns the TLS certificate file path of the first server
func (c *FullConfig) GetTLSCertFile() string {
	for _, svc := range c.Services {
		if len(svc.Dataplane.Servers) > 0 && svc.Dataplane.Servers[0].TLS.Source != nil {
			return svc.Dataplane.Servers[0].TLS.Source.Cert
		}
	}
	return ""
}

// GetTLSKeyFile returns the TLS key file path of the first server
func (c *FullConfig) GetTLSKeyFile() string {
	for _, svc := range c.Services {
		if len(svc.Dataplane.Servers) > 0 && svc.Dataplane.Servers[0].TLS.Source != nil {
			return svc.Dataplane.Servers[0].TLS.Source.Key
		}
	}
	return ""
}
