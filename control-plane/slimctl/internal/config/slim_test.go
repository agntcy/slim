package config

import (
	"os"
	"path/filepath"
	"testing"
)

func TestDefaultSlimConfig(t *testing.T) {
	config := DefaultSlimConfig()

	if config.Endpoint != "127.0.0.1:8080" {
		t.Errorf("expected default endpoint '127.0.0.1:8080', got '%s'", config.Endpoint)
	}

	if !config.TLS.Insecure {
		t.Error("expected default TLS.Insecure to be true")
	}
}

func TestLoadFromFile(t *testing.T) {
	// Create a temporary directory for test files
	tmpDir := t.TempDir()

	tests := []struct {
		name        string
		yamlContent string
		wantErr     bool
		validate    func(*testing.T, *SlimConfig)
	}{
		{
			name: "valid insecure config",
			yamlContent: `endpoint: "0.0.0.0:9090"
tls:
  insecure: true
`,
			wantErr: false,
			validate: func(t *testing.T, c *SlimConfig) {
				if c.Endpoint != "0.0.0.0:9090" {
					t.Errorf("expected endpoint '0.0.0.0:9090', got '%s'", c.Endpoint)
				}
				if !c.TLS.Insecure {
					t.Error("expected TLS.Insecure to be true")
				}
			},
		},
		{
			name: "valid TLS config with cert paths",
			yamlContent: `endpoint: "0.0.0.0:8443"
tls:
  insecure: false
  cert_file: "/path/to/cert.pem"
  key_file: "/path/to/key.pem"
`,
			wantErr: false,
			validate: func(t *testing.T, c *SlimConfig) {
				if c.Endpoint != "0.0.0.0:8443" {
					t.Errorf("expected endpoint '0.0.0.0:8443', got '%s'", c.Endpoint)
				}
				if c.TLS.Insecure {
					t.Error("expected TLS.Insecure to be false")
				}
				if c.TLS.CertFile != "/path/to/cert.pem" {
					t.Errorf("expected cert_file '/path/to/cert.pem', got '%s'", c.TLS.CertFile)
				}
				if c.TLS.KeyFile != "/path/to/key.pem" {
					t.Errorf("expected key_file '/path/to/key.pem', got '%s'", c.TLS.KeyFile)
				}
			},
		},
		{
			name:        "invalid YAML syntax",
			yamlContent: `endpoint: "test\ninvalid: syntax`,
			wantErr:     true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			// Create temporary YAML file
			filePath := filepath.Join(tmpDir, tt.name+".yaml")
			if err := os.WriteFile(filePath, []byte(tt.yamlContent), 0600); err != nil {
				t.Fatalf("failed to create test file: %v", err)
			}

			// Load configuration
			config, err := LoadFromFile(filePath)

			// Check error expectation
			if (err != nil) != tt.wantErr {
				t.Errorf("LoadFromFile() error = %v, wantErr %v", err, tt.wantErr)
				return
			}

			// Run validation function if provided and no error expected
			if !tt.wantErr && tt.validate != nil {
				tt.validate(t, config)
			}
		})
	}
}

func TestLoadFromFile_FileNotFound(t *testing.T) {
	_, err := LoadFromFile("/nonexistent/config.yaml")
	if err == nil {
		t.Error("expected error for nonexistent file, got nil")
	}
}

func TestSlimConfig_Validate(t *testing.T) {
	// Create temporary directory and test certificates
	tmpDir := t.TempDir()
	certFile := filepath.Join(tmpDir, "test-cert.pem")
	keyFile := filepath.Join(tmpDir, "test-key.pem")

	// Create dummy certificate files
	if err := os.WriteFile(certFile, []byte("dummy cert"), 0600); err != nil {
		t.Fatalf("failed to create test cert file: %v", err)
	}
	if err := os.WriteFile(keyFile, []byte("dummy key"), 0600); err != nil {
		t.Fatalf("failed to create test key file: %v", err)
	}

	tests := []struct {
		name    string
		config  *SlimConfig
		wantErr bool
		errMsg  string
	}{
		{
			name: "valid insecure config",
			config: &SlimConfig{
				Endpoint: "127.0.0.1:8080",
				TLS: TLSConfig{
					Insecure: true,
				},
			},
			wantErr: false,
		},
		{
			name: "valid TLS config with existing files",
			config: &SlimConfig{
				Endpoint: "127.0.0.1:8443",
				TLS: TLSConfig{
					Insecure: false,
					CertFile: certFile,
					KeyFile:  keyFile,
				},
			},
			wantErr: false,
		},
		{
			name: "empty endpoint",
			config: &SlimConfig{
				Endpoint: "",
				TLS: TLSConfig{
					Insecure: true,
				},
			},
			wantErr: true,
			errMsg:  "endpoint cannot be empty",
		},
		{
			name: "TLS enabled but missing cert file",
			config: &SlimConfig{
				Endpoint: "127.0.0.1:8443",
				TLS: TLSConfig{
					Insecure: false,
					CertFile: "",
					KeyFile:  keyFile,
				},
			},
			wantErr: true,
			errMsg:  "tls.cert_file is required",
		},
		{
			name: "TLS enabled but missing key file",
			config: &SlimConfig{
				Endpoint: "127.0.0.1:8443",
				TLS: TLSConfig{
					Insecure: false,
					CertFile: certFile,
					KeyFile:  "",
				},
			},
			wantErr: true,
			errMsg:  "tls.key_file is required",
		},
		{
			name: "TLS enabled but cert file does not exist",
			config: &SlimConfig{
				Endpoint: "127.0.0.1:8443",
				TLS: TLSConfig{
					Insecure: false,
					CertFile: "/nonexistent/cert.pem",
					KeyFile:  keyFile,
				},
			},
			wantErr: true,
			errMsg:  "certificate file not found",
		},
		{
			name: "TLS enabled but key file does not exist",
			config: &SlimConfig{
				Endpoint: "127.0.0.1:8443",
				TLS: TLSConfig{
					Insecure: false,
					CertFile: certFile,
					KeyFile:  "/nonexistent/key.pem",
				},
			},
			wantErr: true,
			errMsg:  "key file not found",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := tt.config.Validate()

			if (err != nil) != tt.wantErr {
				t.Errorf("Validate() error = %v, wantErr %v", err, tt.wantErr)
				return
			}

			if tt.wantErr && tt.errMsg != "" {
				if err == nil || !contains(err.Error(), tt.errMsg) {
					t.Errorf("Validate() error = %v, want error containing %q", err, tt.errMsg)
				}
			}
		})
	}
}

func TestSlimConfig_MergeFlags(t *testing.T) {
	tests := []struct {
		name      string
		config    *SlimConfig
		endpoint  *string
		insecure  *bool
		certFile  *string
		keyFile   *string
		wantValue func(*SlimConfig) bool
	}{
		{
			name: "merge endpoint",
			config: &SlimConfig{
				Endpoint: "127.0.0.1:8080",
				TLS:      TLSConfig{Insecure: true},
			},
			endpoint: stringPtr("0.0.0.0:9090"),
			wantValue: func(c *SlimConfig) bool {
				return c.Endpoint == "0.0.0.0:9090"
			},
		},
		{
			name: "merge insecure flag",
			config: &SlimConfig{
				Endpoint: "127.0.0.1:8080",
				TLS:      TLSConfig{Insecure: true},
			},
			insecure: boolPtr(false),
			wantValue: func(c *SlimConfig) bool {
				return !c.TLS.Insecure
			},
		},
		{
			name: "merge cert and key files",
			config: &SlimConfig{
				Endpoint: "127.0.0.1:8443",
				TLS: TLSConfig{
					Insecure: false,
					CertFile: "/old/cert.pem",
					KeyFile:  "/old/key.pem",
				},
			},
			certFile: stringPtr("/new/cert.pem"),
			keyFile:  stringPtr("/new/key.pem"),
			wantValue: func(c *SlimConfig) bool {
				return c.TLS.CertFile == "/new/cert.pem" && c.TLS.KeyFile == "/new/key.pem"
			},
		},
		{
			name: "nil flags do not override",
			config: &SlimConfig{
				Endpoint: "127.0.0.1:8080",
				TLS:      TLSConfig{Insecure: true},
			},
			endpoint: nil,
			insecure: nil,
			wantValue: func(c *SlimConfig) bool {
				return c.Endpoint == "127.0.0.1:8080" && c.TLS.Insecure
			},
		},
		{
			name: "empty string flags do not override",
			config: &SlimConfig{
				Endpoint: "127.0.0.1:8080",
				TLS: TLSConfig{
					Insecure: false,
					CertFile: "/existing/cert.pem",
					KeyFile:  "/existing/key.pem",
				},
			},
			endpoint: stringPtr(""),
			certFile: stringPtr(""),
			keyFile:  stringPtr(""),
			wantValue: func(c *SlimConfig) bool {
				return c.Endpoint == "127.0.0.1:8080" &&
					c.TLS.CertFile == "/existing/cert.pem" &&
					c.TLS.KeyFile == "/existing/key.pem"
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			tt.config.MergeFlags(tt.endpoint, tt.insecure, tt.certFile, tt.keyFile)

			if !tt.wantValue(tt.config) {
				t.Errorf("MergeFlags() did not produce expected config state")
			}
		})
	}
}

// Helper functions

func stringPtr(s string) *string {
	return &s
}

func boolPtr(b bool) *bool {
	return &b
}

func contains(s, substr string) bool {
	return len(s) >= len(substr) && (s == substr || len(s) > len(substr) &&
		(s[:len(substr)] == substr || s[len(s)-len(substr):] == substr ||
			containsMiddle(s, substr)))
}

func containsMiddle(s, substr string) bool {
	for i := 0; i <= len(s)-len(substr); i++ {
		if s[i:i+len(substr)] == substr {
			return true
		}
	}
	return false
}
