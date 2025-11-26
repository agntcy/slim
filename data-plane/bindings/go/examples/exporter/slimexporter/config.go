package slimexporter

import (
	"errors"
)

// Config defines configuration for the Slim exporter
type Config struct {
	// Endpoint is the target URL for exporting telemetry data
	Endpoint string `mapstructure:"endpoint"`

	// Add any custom configuration fields here
	// For example:
	// APIKey string `mapstructure:"api_key"`
	// Timeout time.Duration `mapstructure:"timeout"`
}

// Validate checks if the exporter configuration is valid
func (cfg *Config) Validate() error {
	if cfg.Endpoint == "" {
		return errors.New("endpoint is required")
	}
	return nil
}
