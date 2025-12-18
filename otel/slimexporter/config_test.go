package slimexporter

import (
	"strings"
	"testing"
)

func TestConfig_Validate(t *testing.T) {
	tests := []struct {
		name    string
		config  *Config
		wantErr bool
		errMsg  string
	}{
		{
			name: "valid config with all fields",
			config: &Config{
				SlimEndpoint: "http://localhost:46357",
				LocalName:    "agntcy/test/exporter",
				SharedSecret: "test-secret",
				Sessions: []SessionConfig{
					{
						ChannelName:  "agntcy/test/channel",
						Signals:      []string{"traces", "metrics", "logs"},
						Participants: []string{"agntcy/test/participant1", "agntcy/test/participant2"},
						MlsEnabled:   true,
					},
				},
			},
			wantErr: false,
		},
		{
			name: "valid config with minimal fields",
			config: &Config{
				SharedSecret: "test-secret",
				Sessions: []SessionConfig{
					{
						ChannelName:  "agntcy/test/channel",
						Signals:      []string{"traces"},
						Participants: []string{"agntcy/test/participant1"},
					},
				},
			},
			wantErr: false,
		},
		{
			name: "valid config with empty sessions",
			config: &Config{
				SlimEndpoint: "http://localhost:46357",
				LocalName:    "agntcy/test/exporter",
				SharedSecret: "test-secret",
				Sessions:     []SessionConfig{},
			},
			wantErr: false,
		},
		{
			name: "missing shared secret",
			config: &Config{
				SlimEndpoint: "http://localhost:46357",
				LocalName:    "agntcy/test/exporter",
				Sessions: []SessionConfig{
					{
						ChannelName:  "agntcy/test/channel",
						Signals:      []string{"traces"},
						Participants: []string{"agntcy/test/participant1"},
					},
				},
			},
			wantErr: true,
			errMsg:  "missing shared secret",
		},
		{
			name: "session with missing channel name",
			config: &Config{
				SharedSecret: "test-secret",
				Sessions: []SessionConfig{
					{
						ChannelName:  "",
						Signals:      []string{"traces"},
						Participants: []string{"agntcy/test/participant1"},
					},
				},
			},
			wantErr: true,
			errMsg:  "channel-name is required",
		},
		{
			name: "session with empty signals",
			config: &Config{
				SharedSecret: "test-secret",
				Sessions: []SessionConfig{
					{
						ChannelName:  "agntcy/test/channel",
						Signals:      []string{},
						Participants: []string{"agntcy/test/participant1"},
					},
				},
			},
			wantErr: true,
			errMsg:  "at least one signal must be specified",
		},
		{
			name: "session with invalid signal type",
			config: &Config{
				SharedSecret: "test-secret",
				Sessions: []SessionConfig{
					{
						ChannelName:  "agntcy/test/channel",
						Signals:      []string{"traces", "invalid-signal"},
						Participants: []string{"agntcy/test/participant1"},
					},
				},
			},
			wantErr: true,
			errMsg:  "invalid signal type",
		},
		{
			name: "session with empty participants",
			config: &Config{
				SharedSecret: "test-secret",
				Sessions: []SessionConfig{
					{
						ChannelName:  "agntcy/test/channel",
						Signals:      []string{"traces"},
						Participants: []string{},
					},
				},
			},
			wantErr: true,
			errMsg:  "at least one participant must be specified",
		},
		{
			name: "multiple valid sessions",
			config: &Config{
				SharedSecret: "test-secret",
				Sessions: []SessionConfig{
					{
						ChannelName:  "agntcy/test/channel1",
						Signals:      []string{"traces"},
						Participants: []string{"agntcy/test/participant1"},
						MlsEnabled:   true,
					},
					{
						ChannelName:  "agntcy/test/channel2",
						Signals:      []string{"metrics", "logs"},
						Participants: []string{"agntcy/test/participant2", "agntcy/test/participant3"},
						MlsEnabled:   false,
					},
				},
			},
			wantErr: false,
		},
		{
			name: "valid config with all signal types",
			config: &Config{
				SharedSecret: "test-secret",
				Sessions: []SessionConfig{
					{
						ChannelName:  "agntcy/test/channel",
						Signals:      []string{"traces", "metrics", "logs"},
						Participants: []string{"agntcy/test/participant1"},
					},
				},
			},
			wantErr: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := tt.config.Validate()
			if (err != nil) != tt.wantErr {
				t.Errorf("Config.Validate() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			if tt.wantErr && err != nil && tt.errMsg != "" {
				if !strings.Contains(err.Error(), tt.errMsg) {
					t.Errorf("Config.Validate() error = %v, expected to contain %v", err.Error(), tt.errMsg)
				}
			}
		})
	}
}

func TestConfig_Validate_DefaultValues(t *testing.T) {
	config := &Config{
		SharedSecret: "test-secret",
		Sessions:     []SessionConfig{},
	}

	err := config.Validate()
	if err != nil {
		t.Fatalf("Config.Validate() unexpected error = %v", err)
	}

	// Check that default values are applied
	defaultCfg := createDefaultConfig().(*Config)
	if config.SlimEndpoint != defaultCfg.SlimEndpoint {
		t.Errorf("SlimEndpoint = %v, want %v", config.SlimEndpoint, defaultCfg.SlimEndpoint)
	}
	if config.LocalName != defaultCfg.LocalName {
		t.Errorf("LocalName = %v, want %v", config.LocalName, defaultCfg.LocalName)
	}
}
