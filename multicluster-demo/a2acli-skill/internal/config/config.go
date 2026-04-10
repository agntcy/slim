// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Package config loads a2acli configuration from a YAML file.
package config

import (
	"fmt"
	"os"
	"path/filepath"

	"gopkg.in/yaml.v3"
)

const configFileName = ".a2acli.yaml"

// SpireConfig holds SPIRE Workload API settings for dynamic identity auth.
type SpireConfig struct {
	SocketPath      string   `yaml:"socket-path"`
	TargetSpiffeID  string   `yaml:"target-spiffe-id"`
	JwtAudiences    []string `yaml:"jwt-audiences"`
}

// SlimConfig holds connection settings for the SLIM messaging node.
type SlimConfig struct {
	Endpoint       string      `yaml:"endpoint"`
	LocalName      string      `yaml:"local-name"`
	Secret         string      `yaml:"secret"`
	TLSSkipVerify  bool        `yaml:"tls-skip-verify"`
	Spire          SpireConfig `yaml:"spire"`
}

// Config is the top-level configuration for a2acli.
type Config struct {
	AgentsDir string     `yaml:"agents-dir"`
	Slim      SlimConfig `yaml:"slim"`
}

// Load reads the configuration file from the first location that exists:
//  1. explicitPath if non-empty
//  2. .a2acli.yaml in the current working directory
//  3. ~/.a2acli.yaml in the user's home directory
//
// If no config file is found, an empty Config is returned without error.
func Load(explicitPath string) (*Config, error) {
	path, err := resolve(explicitPath)
	if err != nil {
		return nil, err
	}
	if path == "" {
		return &Config{}, nil
	}

	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("reading config file %q: %w", path, err)
	}

	var cfg Config
	if err := yaml.Unmarshal(data, &cfg); err != nil {
		return nil, fmt.Errorf("parsing config file %q: %w", path, err)
	}
	return &cfg, nil
}

// resolve returns the path of the config file to use, or "" if none is found.
func resolve(explicitPath string) (string, error) {
	if explicitPath != "" {
		if _, err := os.Stat(explicitPath); err != nil {
			return "", fmt.Errorf("config file %q not found: %w", explicitPath, err)
		}
		return explicitPath, nil
	}

	localPath := filepath.Join(".", configFileName)
	if _, err := os.Stat(localPath); err == nil {
		return localPath, nil
	}

	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("could not determine home directory: %w", err)
	}
	homePath := filepath.Join(home, configFileName)
	if _, err := os.Stat(homePath); err == nil {
		return homePath, nil
	}

	return "", nil
}
