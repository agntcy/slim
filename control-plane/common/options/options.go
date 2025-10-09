// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package options

import (
	"time"

	"go.uber.org/zap"
)

// CommonOptions provides the common flags, options, for all the commands.
type CommonOptions struct {
	// Basic auth credentials - username:password format
	BasicAuthCredentials string `yaml:"basic_auth_credentials,omitempty"`

	// SLIM control API endpoint
	Server string `yaml:"server,omitempty"`

	// gRPC request timeout
	Timeout time.Duration `yaml:"timeout,omitempty"`

	// TLS certificate verification
	TLSInsecure bool `yaml:"tls_insecure,omitempty"`

	// CA certificate
	TLSCAFile string `yaml:"tls_ca_file,omitempty"`

	// client TLS certificate
	TLSCertFile string `yaml:"tls_cert_file,omitempty"`

	// client TLS key
	TLSKeyFile string `yaml:"tls_key_file,omitempty"`

	// The logger. For now we only use zap
	Logger *zap.Logger `yaml:"logger,omitempty"`

	// Log file
	LogFile string `yaml:"log_file,omitempty"`

	// Log Level
	LogLevel string `yaml:"log_level,omitempty"`
}

func NewOptions() *CommonOptions {
	return &CommonOptions{}
}
