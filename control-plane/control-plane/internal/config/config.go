package config

import (
	"errors"
	"fmt"
	"os"
	"strings"

	"github.com/rs/zerolog"
	"gopkg.in/yaml.v3"
)

// Application configuration
type ControlPlaneConfig struct {
	Northbound APIConfig `yaml:"northbound"`
	Southbound APIConfig `yaml:"southbound"`
}

type LogConfig struct {
	Level string `yaml:"level"` // Log level, e.g., "debug", "info", "warn", "error"
}

// validate logconfig
func (l LogConfig) Validate() error {
	if l.Level == "" {
		panic("logging.level is required")
	}
	_, err := zerolog.ParseLevel(strings.ToLower(l.Level))
	if err != nil {
		return fmt.Errorf("invalid logging.level: %s", l.Level)
	}
	return nil
}

type APIConfig struct {
	HTTPHost  string      `yaml:"httpHost"`
	HTTPPort  string      `yaml:"httpPort"`
	LogConfig LogConfig   `yaml:"logging"`
	TLS       *TLSConfig  `yaml:"tls"`
	Spire     SpireConfig `yaml:"spire"`
}

type TLSConfig struct {
	UseSpiffe bool   `yaml:"useSpiffe"`
	CertFile  string `yaml:"certFile"`
	KeyFile   string `yaml:"keyFile"`
	CAFile    string `yaml:"caFile"`
}

func (a *TLSConfig) Validate() error {
	if a.UseSpiffe {
		if a.CAFile != "" || a.CertFile != "" || a.KeyFile != "" {
			return errors.New("when useSpiffe is true, certFile, keyFile and caFile are not needed, " +
				"as certificates are provided by SPIRE agent")
		}
		return nil
	}
	if a.CAFile == "" {
		return errors.New("caFile is required when useSpiffe is false")
	}
	if a.CertFile == "" {
		return errors.New("certFile is required when useSpiffe is false")
	}
	return nil
}

func (a *TLSConfig) ServerCertificateIsSet() bool {
	return a.CertFile != "" && a.KeyFile != ""
}

type SpireConfig struct {
	SocketPath string `yaml:"socketPath"`
}

// validate APIConfig
func (a APIConfig) Validate() error {
	if a.HTTPPort == "" {
		return errors.New("HTTPPort is required")
	}
	if a.HTTPHost == "" {
		return errors.New("HTTPHost is required")
	}
	if a.TLS != nil {
		if err := a.TLS.Validate(); err != nil {
			return fmt.Errorf("invalid TLS configuration: %w", err)
		}
		if a.TLS.UseSpiffe && a.Spire.SocketPath == "" {
			return errors.New("spire.socketPath is required when useSpiffe is true")
		}
	}
	return a.LogConfig.Validate()
}

func DefaultConfig() *ControlPlaneConfig {
	return &ControlPlaneConfig{
		APIConfig{
			HTTPHost: "localhost",
			HTTPPort: "50051",
			LogConfig: LogConfig{
				Level: "debug", // Default log level
			},
		},
		APIConfig{
			HTTPHost: "localhost",
			HTTPPort: "50052",
			LogConfig: LogConfig{
				Level: "debug", // Default log level
			},
		},
	}
}

func (c ControlPlaneConfig) OverrideFromFile(file string) *ControlPlaneConfig {
	configFile := file
	if configFile == "" {
		configFile = "config.yaml"
	}
	configData, err := os.ReadFile(configFile)
	if err != nil {
		if os.IsNotExist(err) {
			return &c
		}
		panic(fmt.Sprintf("failed to read config: %v", err))
	}
	fmt.Printf("Using configuration configData: %s\n", configData)
	err = yaml.Unmarshal(configData, &c)
	if err != nil {
		panic(fmt.Sprintf("failed to unmarshal config: %v", err))
	}

	return &c
}

func (c ControlPlaneConfig) OverrideFromEnv() *ControlPlaneConfig {
	c.Northbound.HTTPPort = getEnvStr("NB_API_HTTP_PORT", c.Northbound.HTTPPort)
	c.Northbound.HTTPHost = getEnvStr("NB_API_HTTP_HOST", c.Northbound.HTTPHost)
	c.Northbound.LogConfig.Level = getEnvStr("NB_API_LOG_LEVEL", c.Northbound.LogConfig.Level)
	c.Southbound.HTTPPort = getEnvStr("SB_API_HTTP_PORT", c.Southbound.HTTPPort)
	c.Southbound.HTTPHost = getEnvStr("SB_API_HTTP_HOST", c.Southbound.HTTPHost)
	c.Southbound.LogConfig.Level = getEnvStr("SB_API_LOG_LEVEL", c.Southbound.LogConfig.Level)
	return &c
}

func (c ControlPlaneConfig) Validate() *ControlPlaneConfig {
	if err := c.Northbound.Validate(); err != nil {
		panic(fmt.Sprintf("invalid northbound API configuration: %v", err))
	}
	if err := c.Southbound.Validate(); err != nil {
		panic(fmt.Sprintf("invalid southbound API configuration: %v", err))
	}
	return &c
}

func getEnvStr(varName string, defValue string) string {
	value := os.Getenv(varName)
	if value == "" {
		return defValue
	}
	return value
}
