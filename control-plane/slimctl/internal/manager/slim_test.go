//go:build cgo

package manager

import (
	"fmt"
	"testing"
)

func TestFormatStartError(t *testing.T) {
	tests := []struct {
		name       string
		err        error
		configPath string
		want       string
	}{
		{
			name:       "port already in use",
			err:        fmt.Errorf("SlimError: Message=Address already in use (os error 48)"),
			configPath: "/tmp/config.yaml",
			want:       "failed to start SLIM: port already in use. Use a different --endpoint",
		},
		{
			name:       "tls config error",
			err:        fmt.Errorf("SlimError: Message=missing server cert or key"),
			configPath: "/tmp/config.yaml",
			want:       "failed to start SLIM: TLS config error. Check cert/key paths and permissions",
		},
		{
			name:       "invalid yaml",
			err:        fmt.Errorf("SlimError: Message=Yaml parse error"),
			configPath: "/tmp/config.yaml",
			want:       "failed to start SLIM: invalid YAML in config",
		},
		{
			name:       "invalid config",
			err:        fmt.Errorf("SlimError: Message=missing grpc endpoint"),
			configPath: "/tmp/config.yaml",
			want:       "failed to start SLIM: invalid config: missing grpc endpoint",
		},
		{
			name:       "io error",
			err:        fmt.Errorf("SlimError: Message=Io error: No such file or directory"),
			configPath: "/tmp/missing.yaml",
			want:       "failed to start SLIM: config file not found or unreadable: /tmp/missing.yaml",
		},
		{
			name:       "generic",
			err:        fmt.Errorf("SlimError: Message=something went wrong"),
			configPath: "/tmp/config.yaml",
			want:       "failed to start SLIM: something went wrong",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := formatStartError(tt.err, tt.configPath)
			if got == nil {
				t.Fatalf("expected error, got nil")
			}
			if got.Error() != tt.want {
				t.Fatalf("unexpected error:\n  got:  %q\n  want: %q", got.Error(), tt.want)
			}
		})
	}
}
