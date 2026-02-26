//go:build cgo

package manager

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"go.uber.org/zap"
	"gopkg.in/yaml.v3"

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

// Start starts the local slim instance using bindings' InitializeFromConfigWithError.
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

	if err := validateConfigFile(configPath); err != nil {
		return formatStartError(err, configPath)
	}

	// Initialize SLIM using the bindings' native config loader
	// This reads the YAML file, validates it, and processes ${env:} substitutions
	// All validation and error handling is done by the bindings
	if err := slim.InitializeFromConfigWithError(configPath); err != nil {
		return formatStartError(err, configPath)
	}

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

type slimConfig struct {
	Services map[string]slimService `yaml:"services"`
}

type slimService struct {
	Dataplane *slimDataplane `yaml:"dataplane"`
}

type slimDataplane struct {
	Servers []slimServer `yaml:"servers"`
}

type slimServer struct {
	Endpoint string        `yaml:"endpoint"`
	Auth     *slimAuth     `yaml:"auth"`
	TLS      *slimTLS      `yaml:"tls"`
	Clients  []interface{} `yaml:"clients"`
}

type slimAuth struct {
	Basic *slimBasicAuth `yaml:"basic"`
}

type slimBasicAuth struct {
	Username string `yaml:"username"`
	Password string `yaml:"password"`
}

type slimTLS struct {
	Insecure bool `yaml:"insecure"`
}

func validateConfigFile(configPath string) error {
	data, err := os.ReadFile(configPath)
	if err != nil {
		return fmt.Errorf("io error: %w", err)
	}

	var cfg slimConfig
	decoder := yaml.NewDecoder(bytes.NewReader(data))
	if err := decoder.Decode(&cfg); err != nil {
		return fmt.Errorf("yaml parse error: %w", err)
	}

	var root yaml.Node
	if err := yaml.Unmarshal(data, &root); err != nil {
		return fmt.Errorf("yaml parse error: %w", err)
	}

	servicesNode := mapValue(rootNode(&root), "services")
	if servicesNode == nil || servicesNode.Kind != yaml.MappingNode {
		return fmt.Errorf("invalid configuration: missing services")
	}

	if err := validateDataplaneKeys(servicesNode); err != nil {
		return err
	}

	for _, svc := range cfg.Services {
		if svc.Dataplane == nil {
			continue
		}
		for _, server := range svc.Dataplane.Servers {
			if strings.TrimSpace(server.Endpoint) == "" {
				return fmt.Errorf("missing grpc endpoint")
			}
			if server.Auth != nil && server.Auth.Basic != nil {
				username := strings.TrimSpace(server.Auth.Basic.Username)
				password := strings.TrimSpace(server.Auth.Basic.Password)
				if username == "" || password == "" {
					return fmt.Errorf("invalid configuration: basic auth username and password required")
				}
			}
		}
	}

	return nil
}

func rootNode(root *yaml.Node) *yaml.Node {
	if root.Kind == yaml.DocumentNode && len(root.Content) > 0 {
		return root.Content[0]
	}
	return root
}

func mapValue(node *yaml.Node, key string) *yaml.Node {
	if node == nil || node.Kind != yaml.MappingNode {
		return nil
	}
	for i := 0; i+1 < len(node.Content); i += 2 {
		k := node.Content[i]
		v := node.Content[i+1]
		if k.Kind == yaml.ScalarNode && k.Value == key {
			return v
		}
	}
	return nil
}

func validateDataplaneKeys(servicesNode *yaml.Node) error {
	for i := 0; i+1 < len(servicesNode.Content); i += 2 {
		svcNode := servicesNode.Content[i+1]
		dataplaneNode := mapValue(svcNode, "dataplane")
		if dataplaneNode == nil {
			continue
		}
		if dataplaneNode.Kind != yaml.MappingNode {
			return fmt.Errorf("invalid configuration: dataplane must be a map")
		}
		for j := 0; j+1 < len(dataplaneNode.Content); j += 2 {
			keyNode := dataplaneNode.Content[j]
			if keyNode.Kind != yaml.ScalarNode {
				continue
			}
			key := keyNode.Value
			if key != "servers" && key != "clients" {
				return fmt.Errorf("invalid configuration: unsupported dataplane field %q", key)
			}
		}
	}
	return nil
}

func formatStartError(err error, configPath string) error {
	clean := strings.TrimPrefix(err.Error(), "SlimError: ")
	msg := clean
	if parts := strings.SplitN(clean, "Message=", 2); len(parts) == 2 {
		msg = parts[1]
	}

	errMsg := strings.TrimSpace(msg)
	lowerMsg := strings.ToLower(errMsg)

	if strings.Contains(lowerMsg, "address already in use") ||
		strings.Contains(lowerMsg, "os error 48") {
		return fmt.Errorf("failed to start SLIM: port already in use. Use a different --endpoint")
	}

	if strings.Contains(lowerMsg, "missing server cert or key") ||
		strings.Contains(lowerMsg, "tls config error") ||
		strings.Contains(lowerMsg, "invalid tls version") ||
		strings.Contains(lowerMsg, "invalid pem") ||
		strings.Contains(lowerMsg, "file i/o error") {
		return fmt.Errorf("failed to start SLIM: TLS config error. Check cert/key paths and permissions")
	}

	if strings.Contains(lowerMsg, "missing grpc endpoint") ||
		strings.Contains(lowerMsg, "missing services") ||
		strings.Contains(lowerMsg, "invalid configuration") ||
		strings.Contains(lowerMsg, "yaml parse error") {
		if strings.Contains(lowerMsg, "yaml parse error") {
			return fmt.Errorf("failed to start SLIM: invalid YAML in config")
		}
		return fmt.Errorf("failed to start SLIM: invalid config: %s", strings.SplitN(errMsg, "\n", 2)[0])
	}

	if strings.Contains(lowerMsg, "io error") {
		return fmt.Errorf("failed to start SLIM: config file not found or unreadable: %s", configPath)
	}

	return fmt.Errorf("failed to start SLIM: %s", strings.SplitN(errMsg, "\n", 2)[0])
}
