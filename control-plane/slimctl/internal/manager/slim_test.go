package manager

import (
	"testing"

	"go.uber.org/zap"

	"github.com/agntcy/slim/control-plane/slimctl/internal/config"
)

func TestNewManager_BackwardCompatibility(t *testing.T) {
	logger := zap.NewNop()
	endpoint := "127.0.0.1:8080"
	port := "8080"

	mgr := NewManager(logger, endpoint, port)

	if mgr == nil {
		t.Fatal("NewManager returned nil")
	}

	// Verify the manager has a SlimConfig created from legacy parameters
	m, ok := mgr.(*manager)
	if !ok {
		t.Fatal("NewManager did not return *manager type")
	}

	if m.SlimConfig == nil {
		t.Error("expected SlimConfig to be initialized")
	}

	if m.SlimConfig.Endpoint != endpoint {
		t.Errorf("expected endpoint '%s', got '%s'", endpoint, m.SlimConfig.Endpoint)
	}

	if !m.SlimConfig.TLS.Insecure {
		t.Error("expected legacy manager to use insecure TLS by default")
	}
}

func TestNewManagerWithConfig(t *testing.T) {
	logger := zap.NewNop()
	slimConfig := &config.SlimConfig{
		Endpoint: "0.0.0.0:9090",
		TLS: config.TLSConfig{
			Insecure: false,
			CertFile: "/path/to/cert.pem",
			KeyFile:  "/path/to/key.pem",
		},
	}

	mgr := NewManagerWithConfig(logger, slimConfig)

	if mgr == nil {
		t.Fatal("NewManagerWithConfig returned nil")
	}

	m, ok := mgr.(*manager)
	if !ok {
		t.Fatal("NewManagerWithConfig did not return *manager type")
	}

	if m.SlimConfig != slimConfig {
		t.Error("expected SlimConfig to be the same instance passed to constructor")
	}

	if m.SlimConfig.Endpoint != "0.0.0.0:9090" {
		t.Errorf("expected endpoint '0.0.0.0:9090', got '%s'", m.SlimConfig.Endpoint)
	}

	if m.SlimConfig.TLS.Insecure {
		t.Error("expected TLS to be enabled (Insecure=false)")
	}
}

func TestNewManagerWithConfig_NilLogger(t *testing.T) {
	slimConfig := &config.SlimConfig{
		Endpoint: "127.0.0.1:8080",
		TLS:      config.TLSConfig{Insecure: true},
	}

	mgr := NewManagerWithConfig(nil, slimConfig)

	if mgr == nil {
		t.Fatal("NewManagerWithConfig returned nil")
	}

	m, ok := mgr.(*manager)
	if !ok {
		t.Fatal("NewManagerWithConfig did not return *manager type")
	}

	if m.Logger == nil {
		t.Error("expected Logger to be initialized with no-op logger when nil is passed")
	}
}

func TestManager_ConfigurationPreservation(t *testing.T) {
	tests := []struct {
		name   string
		config *config.SlimConfig
	}{
		{
			name: "insecure config",
			config: &config.SlimConfig{
				Endpoint: "127.0.0.1:8080",
				TLS:      config.TLSConfig{Insecure: true},
			},
		},
		{
			name: "TLS config",
			config: &config.SlimConfig{
				Endpoint: "0.0.0.0:8443",
				TLS: config.TLSConfig{
					Insecure: false,
					CertFile: "/path/to/cert.pem",
					KeyFile:  "/path/to/key.pem",
				},
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			mgr := NewManagerWithConfig(zap.NewNop(), tt.config)
			m, ok := mgr.(*manager)
			if !ok {
				t.Fatal("NewManagerWithConfig did not return *manager type")
			}

			// Verify configuration is preserved
			if m.SlimConfig.Endpoint != tt.config.Endpoint {
				t.Errorf("endpoint not preserved: expected '%s', got '%s'",
					tt.config.Endpoint, m.SlimConfig.Endpoint)
			}

			if m.SlimConfig.TLS.Insecure != tt.config.TLS.Insecure {
				t.Errorf("TLS.Insecure not preserved: expected %v, got %v",
					tt.config.TLS.Insecure, m.SlimConfig.TLS.Insecure)
			}

			if m.SlimConfig.TLS.CertFile != tt.config.TLS.CertFile {
				t.Errorf("TLS.CertFile not preserved: expected '%s', got '%s'",
					tt.config.TLS.CertFile, m.SlimConfig.TLS.CertFile)
			}

			if m.SlimConfig.TLS.KeyFile != tt.config.TLS.KeyFile {
				t.Errorf("TLS.KeyFile not preserved: expected '%s', got '%s'",
					tt.config.TLS.KeyFile, m.SlimConfig.TLS.KeyFile)
			}
		})
	}
}

func TestManager_InterfaceCompliance(t *testing.T) {
	// Verify that *manager implements the Manager interface
	var _ Manager = (*manager)(nil)

	// Verify that both constructors return Manager interface
	m1 := NewManager(zap.NewNop(), "127.0.0.1:8080", "8080")
	if m1 == nil {
		t.Error("NewManager should return non-nil Manager")
	}

	config := &config.SlimConfig{
		Endpoint: "127.0.0.1:8080",
		TLS:      config.TLSConfig{Insecure: true},
	}
	m2 := NewManagerWithConfig(zap.NewNop(), config)
	if m2 == nil {
		t.Error("NewManagerWithConfig should return non-nil Manager")
	}
}
