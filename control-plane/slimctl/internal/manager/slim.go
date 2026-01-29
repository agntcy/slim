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
	FullConfig *config.FullConfig
}

// NewManager creates a new Manager. If logger is nil, a no-op logger is used.
// Deprecated: Use NewManagerWithFullConfig instead.
func NewManager(logger *zap.Logger, endpoint, _ string) Manager {
	if logger == nil {
		logger = zap.NewNop()
	}
	// Convert legacy parameters to FullConfig for backward compatibility
	fullConfig := config.DefaultFullConfig()
	// Update the endpoint in the first service's first server
	for k := range fullConfig.Services {
		svc := fullConfig.Services[k]
		if len(svc.Dataplane.Servers) > 0 {
			svc.Dataplane.Servers[0].Endpoint = endpoint
			svc.Dataplane.Servers[0].TLS.Insecure = true
			fullConfig.Services[k] = svc
		}
		break
	}
	return &manager{
		Logger:     logger,
		FullConfig: fullConfig,
	}
}

// NewManagerWithFullConfig creates a new Manager with a FullConfig. If logger is nil, a no-op logger is used.
func NewManagerWithFullConfig(logger *zap.Logger, fullConfig *config.FullConfig) Manager {
	if logger == nil {
		logger = zap.NewNop()
	}
	return &manager{
		Logger:     logger,
		FullConfig: fullConfig,
	}
}

// Start starts the local slim instance.
func (m *manager) Start(_ context.Context) error {
	m.Logger.Info("Starting slim instance")

	// Convert config to bindings format
	runtimeConfig, tracingConfig, serviceConfigs := m.FullConfig.ToBindingsConfigs()

	// Initialize with production-like configuration
	if err := slim.InitializeWithConfigs(runtimeConfig, tracingConfig, serviceConfigs); err != nil {
		m.Logger.Error("failed to initialize SLIM", zap.Error(err))
		return fmt.Errorf("failed to initialize SLIM: %w", err)
	}

	// Extract server config from services
	serverConfig, err := m.FullConfig.GetServerConfig()
	if err != nil {
		m.Logger.Error("failed to get server config", zap.Error(err))
		return fmt.Errorf("failed to get server config: %w", err)
	}

	endpoint := m.FullConfig.GetEndpoint()

	// Display startup information
	fmt.Printf("🌐 Starting server on %s...\n", endpoint)
	fmt.Printf("   Runtime: %d cores, thread name: %s\n", runtimeConfig.NCores, runtimeConfig.ThreadName)
	fmt.Printf("   Tracing: level=%s\n", tracingConfig.LogLevel)
	if m.FullConfig.IsTLSEnabled() {
		fmt.Println("   TLS enabled")
		fmt.Printf("   Certificate: %s\n", m.FullConfig.GetTLSCertFile())
		fmt.Printf("   Key: %s\n", m.FullConfig.GetTLSKeyFile())
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
		m.Logger.Error("server error", zap.Error(err))
		return fmt.Errorf("server error: %w", err)
	case sig := <-sigChan:
		fmt.Printf("\n\n📋 Received signal: %v\n", sig)
		fmt.Println("Shutting down...")
	}

	return nil
}

// Stop stops the local slim instance.
func (m *manager) Stop(_ context.Context) error {
	m.Logger.Info("Stopping slim instance")
	return nil
}

// Status returns the status of the local slim instance.
func (m *manager) Status(_ context.Context) (string, error) {
	m.Logger.Info("Getting status of slim instance")
	return "unknown", nil
}
