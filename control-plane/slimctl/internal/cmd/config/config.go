package config

import (
	"github.com/spf13/cobra"

	"github.com/agntcy/slim/control-plane/slimctl/internal/cfg"
	"github.com/agntcy/slim/control-plane/slimctl/internal/cmd"
)

func NewConfigCmd(conf *cfg.ConfigData) *cobra.Command {
	cmd := &cobra.Command{
		Use:     "config",
		Short:   "Manage slimctl configuration",
		Long:    `Manage slimctl configuration.`,
		GroupID: cmd.GroupSlimctl,
	}

	cmd.AddCommand(newSetConfigCmd(conf))
	cmd.AddCommand(newListConfigCmd(conf))

	return cmd
}
