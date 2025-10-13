package config

import (
	"github.com/spf13/cobra"

	"github.com/agntcy/slim/control-plane/slimctl/internal/cfg"
)

func NewConfigCmd(conf *cfg.ConfigData) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "config",
		Short: "Manage slimctl configuration",
		Long:  `Manage slimctl configuration.`,
	}

	cmd.AddCommand(newSetConfigCmd(conf))
	cmd.AddCommand(newListConfigCmd(conf))

	return cmd
}
