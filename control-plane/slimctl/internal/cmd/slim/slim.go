package slim

import (
	"context"
	"fmt"

	"github.com/spf13/cobra"

	"github.com/agntcy/slim/control-plane/slimctl/internal/manager"
)

// NewSlimCmd creates the base 'slim' command.
func NewSlimCmd(ctx context.Context, manager manager.Manager) *cobra.Command {
	slimCmd := &cobra.Command{
		Use:     "slim",
		Aliases: []string{"s"},
		Short:   "Manage a local SLIM instance",
		Long:    `Manage a local SLIM instance`,
		GroupID: "slim",
	}
	// Pass manager to subcommands so that they can use it.
	slimCmd.AddCommand(NewStartCmd(manager))
	slimCmd.AddCommand(NewStopCmd(manager))
	slimCmd.AddCommand(NewStatusCmd(manager))
	return slimCmd
}

// NewStartCmd creates the 'start' command.
func NewStartCmd(manager manager.Manager) *cobra.Command {
	return &cobra.Command{
		Use:   "start",
		Short: "Start the SLIM instance",
		Long:  `Start the SLIM instance`,
		RunE: func(cmd *cobra.Command, args []string) error {
			return manager.Start(cmd.Context())
		},
	}
}

// NewStopCmd creates the 'stop' command.
func NewStopCmd(manager manager.Manager) *cobra.Command {
	return &cobra.Command{
		Use:   "stop",
		Short: "Stop the SLIM instance",
		Long:  `Stop the SLIM instance`,
		RunE: func(cmd *cobra.Command, args []string) error {
			return manager.Stop(cmd.Context())
		},
	}
}

// NewStatusCmd creates the 'status' command.
func NewStatusCmd(manager manager.Manager) *cobra.Command {
	return &cobra.Command{
		Use:   "status",
		Short: "Get the status of the SLIM instance",
		Long:  `Get the status of the SLIM instance`,
		RunE: func(cmd *cobra.Command, args []string) error {
			status, err := manager.Status(cmd.Context())
			if err != nil {
				return fmt.Errorf("failed to get status: %w", err)
			}
			fmt.Printf("SLIM instance status: %s\n", status)
			return nil
		},
	}
}
