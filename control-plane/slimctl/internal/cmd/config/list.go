package config

import (
	"fmt"

	"github.com/spf13/cobra"
	"gopkg.in/yaml.v3"

	"github.com/agntcy/slim/control-plane/slimctl/internal/cfg"
)

func newListConfigCmd(conf *cfg.ConfigData) *cobra.Command {
	return &cobra.Command{
		Use:     "list",
		Aliases: []string{"ls"},
		Short:   "List configuration values",
		Long:    `List configuration values.`,
		RunE: func(_ *cobra.Command, _ []string) error {
			appConf, err := cfg.LoadConfig(conf.Fs)
			if err != nil {
				return err
			}
			err = prettyPrintYAML(appConf)
			if err != nil {
				return err
			}
			return nil
		},
	}
}

func prettyPrintYAML(v interface{}) error {
	out, err := yaml.Marshal(v)
	if err != nil {
		return err
	}
	fmt.Println(string(out))
	return nil
}
