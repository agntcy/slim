package config

import (
	"fmt"

	"github.com/spf13/cobra"
	"gopkg.in/yaml.v3"
)

func newListConfigCmd() *cobra.Command {
	return &cobra.Command{
		Use:     "list",
		Aliases: []string{"ls"},
		Short:   "List configuration values",
		Long:    `List configuration values.`,
		RunE: func(cmd *cobra.Command, args []string) error {
			conf, err := loadConfig()
			if err != nil {
				return err
			}
			err = prettyPrintYAML(conf)
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
