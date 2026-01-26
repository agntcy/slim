package manager

import (
	"context"
	"fmt"
	"os"
	"os/signal"
	"syscall"
	"time"

	"go.uber.org/zap"

	slim "github.com/agntcy/slim-bindings-go"
	"github.com/agntcy/slim/control-plane/slimctl/internal/config"
)

// Manager defines management operations for a local slim instance.
type Manager interface {
	Start(ctx context.Context) error
	Stop(ctx context.Context) error
	Status(ctx context.Context) (string, error)
}

// Service is the default implementation of Manager.
type manager struct {
	Logger     *zap.Logger
	SlimConfig *config.SlimConfig
}

// NewManager creates a new Manager. If logger is nil, a no-op logger is used.
// Deprecated: Use NewManagerWithConfig instead.
func NewManager(logger *zap.Logger, endpoint, _ string) Manager {
	if logger == nil {
		logger = zap.NewNop()
	}
	// Convert legacy parameters to SlimConfig for backward compatibility
	slimConfig := &config.SlimConfig{
		Endpoint: endpoint,
		TLS: config.TLSConfig{
			Insecure: true, // Legacy behavior was insecure
		},
	}
	return &manager{
		Logger:     logger,
		SlimConfig: slimConfig,
	}
}

// NewManagerWithConfig creates a new Manager with a SlimConfig. If logger is nil, a no-op logger is used.
func NewManagerWithConfig(logger *zap.Logger, slimConfig *config.SlimConfig) Manager {
	if logger == nil {
		logger = zap.NewNop()
	}
	return &manager{
		Logger:     logger,
		SlimConfig: slimConfig,
	}
}

// Start starts the local slim instance.
func (s *manager) Start(_ context.Context) error {
	s.Logger.Info("Starting slim instance")

	// Initialize crypto
	slim.InitializeWithDefaults()

	// Create server configuration from SlimConfig
	serverConfig, err := s.SlimConfig.ToServerConfig()
	if err != nil {
		s.Logger.Error("failed to create server config", zap.Error(err))
		return fmt.Errorf("failed to create server config: %w", err)
	}

	endpoint := s.SlimConfig.Endpoint

	// Display startup information
	fmt.Printf("🌐 Starting server on %s...\n", endpoint)
	if !s.SlimConfig.TLS.Insecure {
		fmt.Println("   TLS enabled")
		fmt.Printf("   Certificate: %s\n", s.SlimConfig.TLS.CertFile)
		fmt.Printf("   Key: %s\n", s.SlimConfig.TLS.KeyFile)
	} else {
		fmt.Println("   Running in insecure mode (no TLS)")
	}
	fmt.Println("   Waiting for clients to connect...")
	fmt.Println()

	// Run server in goroutine (it blocks)
	serverErr := make(chan error, 1)
	go func() {
		if err := slim.GetGlobalService().RunServer(serverConfig); err != nil {
			serverErr <- err
		}
	}()

	// Give server a moment to start
	time.Sleep(100 * time.Millisecond)

	fmt.Println("✅ Server running and listening")
	fmt.Println()
	fmt.Printf("📡 Endpoint: %s\n", endpoint)
	fmt.Println()
	fmt.Println("Press Ctrl+C to stop")

	// Wait for interrupt or error
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)

	select {
	case err := <-serverErr:
		s.Logger.Error("server error", zap.Error(err))
		return fmt.Errorf("server error: %w", err)
	case sig := <-sigChan:
		fmt.Printf("\n\n📋 Received signal: %v\n", sig)
		fmt.Println("Shutting down...")
	}

	return nil
}

// Stop stops the local slim instance.
func (s *manager) Stop(_ context.Context) error {
	s.Logger.Info("Stopping slim instance")
	return nil
}

// Status returns the status of the local slim instance.
func (s *manager) Status(_ context.Context) (string, error) {
	s.Logger.Info("Getting status of slim instance")
	return "unknown", nil
}
