package config

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/spf13/cobra"
	"gopkg.in/yaml.v3"

	"github.com/agntcy/slim/control-plane/common/options"
	"github.com/agntcy/slim/control-plane/slimctl/internal/utils"
)

func NewConfigCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "config",
		Short: "Manage slimctl configuration",
		Long:  `Manage slimctl configuration.`,
	}

	cmd.AddCommand(newSetConfigCmd())
	cmd.AddCommand(newListConfigCmd())

	return cmd
}

func getConfigPath() (string, error) {
	home, err := os.UserHomeDir()

	if err != nil {
		return "", err
	}
	return filepath.Join(home, ".config", "slimctl"), nil
}

func getConfigFile() (string, error) {
	path, err := getConfigPath()
	if err != nil {
		return "", err
	}

	return filepath.Join(path, "config.yaml"), nil
}

func saveConfig(opts *options.CommonOptions) error {
	configData, err := yaml.Marshal(opts)
	if err != nil {
		return err
	}

	configPath, err := getConfigPath()
	err = os.MkdirAll(configPath, 0755)

	if err != nil {
		return err
	}

	configFile, _ := getConfigFile()
	return utils.SaveFile(configFile, configData)
}

func loadConfig() (*options.CommonOptions, error) {
	configFile, err := getConfigFile()
	if err != nil {
		fmt.Printf("Error getting config file: %v\n", err)
		return nil, err
	}

	if !utils.FileExists(configFile) {
		return &options.CommonOptions{}, nil
	}

	configData, err := os.ReadFile(configFile)
	if err != nil {
		fmt.Printf("Error reading config file123: %v\n", err)
		return nil, err
	}
	var opts options.CommonOptions
	if err := yaml.Unmarshal(configData, &opts); err != nil {
		return nil, err
	}
	return &opts, nil
}

// func saveConfig(opts *options.CommonOptions) error {
// 	configData, err := yaml.Marshal(opts)
// 	if err != nil {
// 		return err
// 	}
// 	configFile, err := getConfigFile()
// 	if err != nil {
// 		return err
// 	}
// 	return os.WriteFile(configFile, configData, 0644)
// }

// type config struct {
// 	CommonOpts *options.CommonOptions `yaml:"common_opts"`
// }
