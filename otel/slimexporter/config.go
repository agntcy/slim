package slimexporter

import "errors"

// Config defines configuration for the Slim exporter
type Config struct {
	// Slim endpoint where to connect
	SlimEndpoint string `mapstructure:"endpoint"`

	// Local name in the form org/ns/service
	// default = agntcy/otel/exporter
	LocalName string `mapstructure:"local-name"`

	// Shared Secret
	SharedSecret string `mapstructure:"shared-secret"`

	// List of sessions/channels to create
	Sessions []SessionConfig `mapstructure:"sessions"`
}

// SessionConfig defines configuration for a single session/channel
type SessionConfig struct {
	// Channel name in the form org/ns/service
	// this is the base name, actual channels will be
	// channel-name-traces, channel-name-metrics, channel-name-logs
	ChannelName string `mapstructure:"channel-name"`

	// Signals to export on this channels (traces, metrics, logs)
	// signals will be added to the channel name as suffix
	Signals []string `mapstructure:"signals"`

	// List of participants to invite to the channels
	Participants []string `mapstructure:"participants"`

	// Flag to enable or disable MLS for these sessions
	MlsEnabled bool `mapstructure:"mls-enabled"`
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

	// Validate each session (the list can be empty)
	for i, session := range cfg.Sessions {
		if session.ChannelName == "" {
			return errors.New("channel-name is required for session " + string(i))
		}
		if len(session.Signals) == 0 {
			return errors.New("at least one signal must be specified for session " + session.ChannelName)
		}
		// Validate signal types
		for _, signal := range session.Signals {
			if signal != "traces" && signal != "metrics" && signal != "logs" {
				return errors.New("invalid signal type: " + signal + " (must be traces, metrics, or logs)")
			}
		}
		if len(session.Participants) == 0 {
			return errors.New("at least one participant must be specified for session " + session.ChannelName)
		}
	}

	return nil
}
