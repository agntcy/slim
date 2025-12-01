package slim

import (
	"github.com/spf13/cobra"
)

// NewSlimCmd creates the base 'slim' command.
func NewSlimCmd() *cobra.Command {
	slimCmd := &cobra.Command{
		Use:     "slim",
		Aliases: []string{"s"},
		Short:   "Manage a local SLIM instance",
		Long:    `Manage a local SLIM instance`,
		GroupID: "slim",
	}

	// Add subcommands
	slimCmd.AddCommand(NewStartCmd())
	slimCmd.AddCommand(NewStopCmd())
	slimCmd.AddCommand(NewStatusCmd())

	return slimCmd
}

// NewStartCmd creates the 'start' command.
func NewStartCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "start",
		Short: "Start the SLIM instance",
		Long:  `Start the SLIM instance`,
	}
}

// NewStopCmd creates the 'stop' command.
func NewStopCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "stop",
		Short: "Stop the SLIM instance",
		Long:  `Stop the SLIM instance`,
	}
}

// NewStatusCmd creates the 'status' command.
func NewStatusCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "status",
		Short: "Get the status of the SLIM instance",
		Long:  `Get the status of the SLIM instance`,
	}
}
