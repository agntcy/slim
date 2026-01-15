package controller

import (
	"github.com/spf13/cobra"

	"github.com/agntcy/slim/control-plane/slimctl/internal/cfg"
	"github.com/agntcy/slim/control-plane/slimctl/internal/cmd"
)

// NewControllerCmd creates the base 'controller' command.
// All operations communicating with the SLIM Controller should be added as subcommands to this command.
func NewControllerCmd(cfg *cfg.ConfigData) *cobra.Command {
	ctrlCmd := &cobra.Command{
		Use:     "controller",
		Aliases: []string{"c", "ctrl"},
		Short:   "Controller Commands",
		Long:    `Command group for operations communicating with the SLIM Controller`,
		GroupID: cmd.GroupController,
	}

	// Add subcommands
	ctrlCmd.AddCommand(NewNodeCmd(cfg.AppConfig.CommonOpts))
	ctrlCmd.AddCommand(NewConnectionCmd(cfg.AppConfig.CommonOpts))
	ctrlCmd.AddCommand(NewRouteCmd(cfg.AppConfig.CommonOpts))
	ctrlCmd.AddCommand(NewChannelCmd(cfg.AppConfig.CommonOpts))
	ctrlCmd.AddCommand(NewParticipantCmd(cfg.AppConfig.CommonOpts))

	return ctrlCmd
}
