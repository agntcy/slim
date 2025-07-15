package config

import (
	"fmt"
	"os"
	"strconv"
	"strings"

	"gopkg.in/yaml.v3"
)

// Application configuration
type nbAPIconfig struct {
	HttpHost string `yaml:"httpHost"`
	HttpPort string `yaml:"httpPort"`
}

func DefaultConfig() *nbAPIconfig {
	return &nbAPIconfig{
		HttpHost: "localhost",
		HttpPort: "50051",
	}
}

func (c nbAPIconfig) OverrideFromFile(file string) *nbAPIconfig {
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

func (c nbAPIconfig) OverrideFromEnv() *nbAPIconfig {
	c.HttpPort = getEnvStr("NB_API_HTTP_PORT", c.HttpPort)
	c.HttpHost = getEnvStr("NB_API_HTTP_HOST", c.HttpHost)
	return &c
}

func (c nbAPIconfig) Validate() *nbAPIconfig {
	if c.HttpPort == "" {
		panic("configuration invalid: HttpPort is required")
	}
	if c.HttpHost == "" {
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
