package manager

import (
	"context"
	"fmt"
	"log"
	"os"
	"os/signal"
	"syscall"
	"time"

	slim "github.com/agntcy/slim/bindings/generated/slim_uniffi"
	"go.uber.org/zap"
)

// Manager defines management operations for a local slim instance.
type Manager interface {
	Start(ctx context.Context) error
	Stop(ctx context.Context) error
	Status(ctx context.Context) (string, error)
}

// Service is the default implementation of Manager.
type manager struct {
	Logger   *zap.Logger
	Secret   string
	Endpoint string
}

// NewManager creates a new Manager. If logger is nil, a no-op logger is used.
func NewManager(logger *zap.Logger, secret string, endpoint string) Manager {
	if logger == nil {
		logger = zap.NewNop()
	}
	return &manager{Logger: logger, Secret: secret, Endpoint: endpoint}
}

// Start starts the local slim instance.
func (s *manager) Start(ctx context.Context) error {
	s.Logger.Info("Starting slim instance")

	// Initialize crypto
	slim.InitializeCryptoProvider()

	// Create server app
	serverName := slim.Name{
		Components: []string{"system", "server", "node"},
		Id:         nil,
	}

	app, err := slim.CreateAppWithSecret(serverName, s.Secret)
	if err != nil {
		s.Logger.Error("failed to create server app", zap.Error(err))
		return fmt.Errorf("failed to create server app: %w", err)
	}
	defer app.Destroy()

	fmt.Printf("✅ Server app created (ID: %d)\n", app.Id())

	// Start server
	config := slim.ServerConfig{
		Endpoint: s.Endpoint,
		Tls:      slim.TlsConfig{Insecure: true},
	}

	fmt.Printf("🌐 Starting server on %s...\n", s.Endpoint)
	fmt.Println("   Waiting for clients to connect...")
	fmt.Println()

	// Run server in goroutine (it blocks)
	serverErr := make(chan error, 1)
	go func() {
		if err := app.RunServer(config); err != nil {
			serverErr <- err
		}
	}()

	// Give server a moment to start
	time.Sleep(100 * time.Millisecond)

	fmt.Println("✅ Server running and listening")
	fmt.Println()
	fmt.Println("📡 Clients can now connect")
	fmt.Println()
	fmt.Println("Press Ctrl+C to stop")

	// Wait for interrupt or error
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)

	select {
	case err := <-serverErr:
		log.Fatalf("Server error: %v", err)
	case sig := <-sigChan:
		fmt.Printf("\n\n📋 Received signal: %v\n", sig)
		fmt.Println("Shutting down...")
	}

	return nil
}

// Stop stops the local slim instance.
func (s *manager) Stop(ctx context.Context) error {
	s.Logger.Info("Stopping slim instance")
	return nil
}

// Status returns the status of the local slim instance.
func (s *manager) Status(ctx context.Context) (string, error) {
	s.Logger.Info("Getting status of slim instance")
	return "unknown", nil
}
