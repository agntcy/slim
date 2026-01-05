// Package config provides configuration types and constants for SLIM example applications.
package config

import (
	"errors"
	"fmt"
	"strings"
	"time"
)

// Default configuration values.
const (
	// HTTP server defaults
	DefaultHTTPPort    = 8081
	DefaultSSEBufferSize = 100
	CookieMaxAge       = 86400 * 7 // 7 days in seconds

	// SLIM connection defaults
	DefaultSlimEndpoint = "http://localhost:46357"
	DefaultSharedSecret = "demo-shared-secret-min-32-chars!!"

	// SLIM timeouts (milliseconds)
	GetMessageTimeoutMs      = 1000
	ListenSessionTimeoutMs   = 5000
	SessionEstablishDelayMs  = 100

	// Invitation management
	DefaultInviteTimeoutSec = 60
	InviteCleanupInterval   = 5 * time.Second
)

// SessionEstablishDelay is the time to wait after creating a session before starting the receiver.
var SessionEstablishDelay = time.Duration(SessionEstablishDelayMs) * time.Millisecond

// Config holds the application configuration.
type Config struct {
	// HTTPPort is the port for the HTTP server.
	HTTPPort int

	// SlimEndpoint is the SLIM server address.
	SlimEndpoint string

	// SharedSecret is the authentication secret for SLIM.
	SharedSecret string

	// LocalID is the local SLIM identity (org/namespace/app format).
	LocalID string

	// InviteTimeout is the timeout for pending invitations in seconds (sessionclient only).
	InviteTimeout int
}

// NewConfig creates a new Config with default values.
func NewConfig() *Config {
	return &Config{
		HTTPPort:      DefaultHTTPPort,
		SlimEndpoint:  DefaultSlimEndpoint,
		SharedSecret:  DefaultSharedSecret,
		InviteTimeout: DefaultInviteTimeoutSec,
	}
}

// Validate checks if the configuration is valid.
func (c *Config) Validate() error {
	if c.HTTPPort <= 0 || c.HTTPPort > 65535 {
		return fmt.Errorf("invalid HTTP port: %d", c.HTTPPort)
	}

	if c.SlimEndpoint == "" {
		return errors.New("SLIM endpoint is required")
	}

	if c.LocalID == "" {
		return errors.New("local ID is required")
	}

	// Validate LocalID format (org/namespace/app)
	parts := strings.Split(c.LocalID, "/")
	if len(parts) != 3 {
		return fmt.Errorf("invalid local ID format: %s (expected org/namespace/app)", c.LocalID)
	}
	for i, part := range parts {
		if part == "" {
			return fmt.Errorf("invalid local ID: part %d is empty", i+1)
		}
	}

	if c.InviteTimeout < 0 {
		return fmt.Errorf("invalid invite timeout: %d", c.InviteTimeout)
	}

	return nil
}

// CookieName returns the cookie name for the given port.
func (c *Config) CookieName(prefix string) string {
	return fmt.Sprintf("%s_user_%d", prefix, c.HTTPPort)
}
