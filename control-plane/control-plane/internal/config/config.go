package config

import (
	"fmt"
	"os"
	"strconv"
	"strings"

	"gopkg.in/yaml.v3"
)

// Application configuration
type controlPlaneConfig struct {
	Northbound nbAPIconfig `yaml:"northbound"`
	Southbound sbAPIconfig `yaml:"southbound"`
}

type nbAPIconfig struct {
	HttpHost string `yaml:"httpHost"`
	HttpPort string `yaml:"httpPort"`
}

type sbAPIconfig struct {
	HttpHost string `yaml:"httpHost"`
	HttpPort string `yaml:"httpPort"`
}

func DefaultConfig() *controlPlaneConfig {
	return &controlPlaneConfig{
		nbAPIconfig{
			HttpHost: "localhost",
			HttpPort: "50051",
		},
		sbAPIconfig{
			HttpHost: "localhost",
			HttpPort: "50052",
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
	c.Southbound.HttpPort = getEnvStr("SB_API_HTTP_PORT", c.Southbound.HttpPort)
	c.Southbound.HttpHost = getEnvStr("SB_API_HTTP_HOST", c.Southbound.HttpHost)
	return &c
}

func (c controlPlaneConfig) Validate() *controlPlaneConfig {
	if c.Northbound.HttpPort == "" {
		panic("configuration invalid: HttpPort is required")
	}
	if c.Northbound.HttpHost == "" {
		panic("configuration invalid: HttpHost is required")
	}
	if c.Southbound.HttpPort == "" {
		panic("configuration invalid: HttpPort is required")
	}
	if c.Southbound.HttpHost == "" {
		panic("configuration invalid: HttpHost is required")
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
