package cfg

import (
	"os"
	"path/filepath"

	"github.com/spf13/afero"
	"gopkg.in/yaml.v3"

	"github.com/agntcy/slim/control-plane/common/options"
	"github.com/agntcy/slim/control-plane/slimctl/internal/utils"
)

type AppConfig struct {
	CommonOpts *options.CommonOptions `yaml:"common_opts"`
}

func newAppConfig() *AppConfig {
	return &AppConfig{
		CommonOpts: options.NewOptions(),
	}
}

type ConfigData struct {
	Fs        afero.Fs
	AppConfig *AppConfig
}

func NewConfigData(fs afero.Fs) *ConfigData {
	return &ConfigData{
		Fs:        fs,
		AppConfig: newAppConfig(),
	}
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

func (conf *ConfigData) SaveConfig() error {
	configData, err := yaml.Marshal(conf.AppConfig)
	if err != nil {
		return err
	}

	configPath, err := getConfigPath()
	if err != nil {
		return err
	}
	err = os.MkdirAll(configPath, 0755)

	if err != nil {
		return err
	}

	configFile, err := getConfigFile()
	if err != nil {
		return err
	}
	return utils.SaveFile(conf.Fs, configFile, configData)
}

func LoadConfig(fs afero.Fs) (*AppConfig, error) {
	configFile, err := getConfigFile()
	if err != nil {
		return nil, err
	}

	if ok, _ := afero.Exists(fs, configFile); !ok {
		return newAppConfig(), nil
	}

	appConfigData, err := afero.ReadFile(fs, configFile)
	if err != nil {
		return nil, err
	}

	var appConf AppConfig
	if err := yaml.Unmarshal(appConfigData, &appConf); err != nil {
		return nil, err
	}
	return &appConf, nil
}

func MapFlagToConfigFunc() func(key string, value string) (string, interface{}) {
	return func(key string, value string) (string, interface{}) {
		// Map specific flags to their config keys
		switch key {
		case "basic_auth_creds":
			return "common_opts.basic_auth_creds", value
		case "server":
			return "common_opts.server", value
		case "timeout":
			return "common_opts.timeout", value
		case "tls.insecure":
			return "common_opts.tls_insecure", value
		case "tls.ca_file":
			return "common_opts.tls_ca_file", value
		case "tls.cert_file":
			return "common_opts.tls_cert_file", value
		case "tls.key_file":
			return "common_opts.tls_key_file", value
		default:
			return key, value
		}
	}
}
