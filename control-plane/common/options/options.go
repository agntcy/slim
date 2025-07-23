// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package options

import (
	"time"

	"go.uber.org/zap"
)

// CommonOptions provides the common flags, options, for all the commands.
type CommonOptions struct {
	// SLIM control API endpoint
	Server string

	// gRPC request timeout
	Timeout time.Duration

	// TLS certificate verification
	TLSInsecure bool

	// CA certificate
	TLSCAFile string

	// client TLS certificate
	TLSCertFile string

	// client TLS key
	TLSKeyFile string

	// The logger. For now we only use zap
	Logger *zap.Logger

	// Log file
	LogFile string

	// Log Level
	LogLevel string
}

func NewOptions() *CommonOptions {
	return &CommonOptions{}
}
