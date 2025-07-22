package config

import (
	"errors"
	"fmt"
	"os"
	"strconv"
	"strings"

	"github.com/rs/zerolog"
	"gopkg.in/yaml.v3"
)

// Application configuration
type controlPlaneConfig struct {
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
	HttpHost  string    `yaml:"httpHost"`
	HttpPort  string    `yaml:"httpPort"`
	LogConfig LogConfig `yaml:"logging"`
}

// validate APIConfig
func (a APIConfig) Validate() error {
	if a.HttpPort == "" {
		return errors.New("HttpPort is required")
	}
	if a.HttpHost == "" {
		return errors.New("HttpHost is required")
	}
	return a.LogConfig.Validate()
}

func DefaultConfig() *controlPlaneConfig {
	return &controlPlaneConfig{
		APIConfig{
			HttpHost: "localhost",
			HttpPort: "50051",
			LogConfig: LogConfig{
				Level: "debug", // Default log level
			},
		},
		APIConfig{
			HttpHost: "localhost",
			HttpPort: "50052",
			LogConfig: LogConfig{
				Level: "debug", // Default log level
			},
		},
	}
}

func (c controlPlaneConfig) OverrideFromFile(file string) *controlPlaneConfig {
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

func (c controlPlaneConfig) OverrideFromEnv() *controlPlaneConfig {
	c.Northbound.HttpPort = getEnvStr("NB_API_HTTP_PORT", c.Northbound.HttpPort)
	c.Northbound.HttpHost = getEnvStr("NB_API_HTTP_HOST", c.Northbound.HttpHost)
	c.Northbound.LogConfig.Level = getEnvStr("NB_API_LOG_LEVEL", c.Northbound.LogConfig.Level)
	c.Southbound.HttpPort = getEnvStr("SB_API_HTTP_PORT", c.Southbound.HttpPort)
	c.Southbound.HttpHost = getEnvStr("SB_API_HTTP_HOST", c.Southbound.HttpHost)
	c.Southbound.LogConfig.Level = getEnvStr("SB_API_LOG_LEVEL", c.Southbound.LogConfig.Level)
	return &c
}

func (c controlPlaneConfig) Validate() *controlPlaneConfig {
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

func getEnvBool(varName string, defValue bool) bool {
	value := os.Getenv(varName)
	if value == "" {
		return defValue
	}
	value = strings.ToLower(value)
	if value == "1" || value == "true" || value == "on" {
		return true
	}
	if value == "0" || value == "false" || value == "off" {
		return false
	}
	return defValue
}

func getEnvInt(varName string, defValue int) int {
	value := os.Getenv(varName)
	if value == "" {
		return defValue
	}

	intVal, err := strconv.Atoi(value)
	if err != nil {
		return defValue
	}
	return intVal
}
