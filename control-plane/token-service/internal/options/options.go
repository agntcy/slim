// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package options

import (
	"go.uber.org/zap"
)

// CommonOptions provides the common flags, options, for all the commands.
type CommonOptions struct {
	// The configuration file, if used
	ConfigFile string

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
