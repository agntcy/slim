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

// manager is the default implementation of Manager.
type manager struct {
	Logger        *zap.Logger
	ConfigManager *config.Manager
}

// NewManager creates a new Manager with a config.Manager. If logger is nil, a no-op logger is used.
func NewManager(logger *zap.Logger, configMgr *config.Manager) Manager {
	if logger == nil {
		logger = zap.NewNop()
	}
	return &manager{
		Logger:        logger,
		ConfigManager: configMgr,
	}
}

// Start starts the local slim instance using bindings' InitializeFromConfig.
// All configuration validation and processing is delegated to the bindings library.
func (m *manager) Start(_ context.Context) error {
	m.Logger.Info("Starting slim instance")

	// Get configuration path (original or temporary with env var refs)
	// No validation - just get the path
	configPath, _ := m.ConfigManager.GetConfigPath()

	// Ensure cleanup of temporary files
	defer func() {
		if err := m.ConfigManager.Cleanup(); err != nil {
			m.Logger.Warn("failed to cleanup temporary config", zap.Error(err))
		}
	}()

	m.Logger.Info("Initializing SLIM from configuration",
		zap.String("config_path", configPath))

	// Initialize SLIM using the bindings' native config loader
	// This reads the YAML file, validates it, and processes ${env:} substitutions
	// All validation and error handling is done by the bindings
	// The bindings will panic with a descriptive error if the config is invalid
	slim.InitializeFromConfig(configPath)

	// The server is already running from InitializeFromConfig
	// Just need to display status and wait for signal
	time.Sleep(100 * time.Millisecond)

	fmt.Println("SLIM dataplane running")
	fmt.Println("   Configuration: ", configPath)
	fmt.Println("Press Ctrl+C to stop")

	// Wait for interrupt signal
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)

	sig := <-sigChan
	fmt.Printf("\n\n📋 Received signal: %v\n", sig)
	fmt.Println("Shutting down...")

	_ = slim.GetGlobalService().Shutdown()

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
