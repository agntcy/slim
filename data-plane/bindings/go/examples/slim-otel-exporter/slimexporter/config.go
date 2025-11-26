package slimexporter

import "errors"

// Config defines configuration for the Slim exporter
type Config struct {
	// Slim endpoint where to connect
	SlimEndpoint string `mapstructure:"endpoint"`

	// Local name in the for org/ns/service
	// dafual = agntcy/otel/exporter
	LocalName string `mapstructure:"local-name"`

	// Channel name in the form org/ns/service
	// the signal type is appended as a suffix to the name
	// default agntcy/otel/telemetry-*
	ChannelName string `mapstructure:"channel-name"`

	// Shared Secret
	SharedSecret string `mapstructure:"shared-secret"`

	// Flag to enable or disable MLS
	MlsEnabled bool `mapstructure:"mls-enabled"`

	// List of participants to invite
	ParticipantsList []string `mapstructure:"participants-list"`
}

// Validate checks if the exporter configuration is valid
func (cfg *Config) Validate() error {
	if cfg.SharedSecret == "" {
		return errors.New("missing shared secret")
	}

	defaultCfg := createDefaultConfig().(*Config)
	if cfg.SlimEndpoint == "" {
		cfg.SlimEndpoint = defaultCfg.SlimEndpoint
	}

	if cfg.LocalName == "" {
		cfg.LocalName = defaultCfg.LocalName
	}

	if cfg.ChannelName == "" {
		cfg.ChannelName = defaultCfg.ChannelName
	}

	return nil
}
