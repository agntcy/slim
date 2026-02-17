package slim

import (
	"context"

	"github.com/spf13/cobra"

	"github.com/agntcy/slim/control-plane/slimctl/internal/cfg"
	"github.com/agntcy/slim/control-plane/slimctl/internal/cmd"
	"github.com/agntcy/slim/control-plane/slimctl/internal/config"
	"github.com/agntcy/slim/control-plane/slimctl/internal/manager"
)

// NewSlimCmd creates the base 'slim' command.
func NewSlimCmd(_ context.Context, appConfig *cfg.AppConfig) *cobra.Command {
	slimCmd := &cobra.Command{
		Use:     "slim",
		Aliases: []string{"s"},
		Short:   "Manage a local SLIM instance",
		Long:    `Manage a local SLIM instance`,
		GroupID: cmd.GroupSlim,
	}
	// Pass appConfig to subcommands so that they can create managers with proper configuration.
	slimCmd.AddCommand(NewStartCmd(appConfig))
	// slimCmd.AddCommand(NewStopCmd(appConfig))
	// slimCmd.AddCommand(NewStatusCmd(appConfig))
	return slimCmd
}

// NewStartCmd creates the 'start' command.
func NewStartCmd(appConfig *cfg.AppConfig) *cobra.Command {
	var configFile string
	var endpoint string

	startCmd := &cobra.Command{
		Use:   "start",
		Short: "Start the SLIM instance",
		Long: `Start the SLIM instance with production configuration.

Configuration is loaded using the SLIM bindings' native InitializeFromConfig,
which handles all validation and processing.

The --endpoint flag can optionally override the server endpoint by setting the
SLIM_ENDPOINT environment variable, which can be referenced in config files
using ${env:SLIM_ENDPOINT} syntax.

For example configurations, see the data-plane/config/ directory in the
repository, which contains production-ready SLIM configurations.

Log level can be controlled via the RUST_LOG environment variable.

All configuration validation is performed by the bindings library.`,
		SilenceErrors: true,
		SilenceUsage:  true,
		RunE: func(c *cobra.Command, _ []string) error {
			// Create config manager - just manages paths, no validation
			configMgr := config.New(configFile)

			// Set SLIM_ENDPOINT environment variable if --endpoint flag is provided
			var endpointPtr *string
			if c.Flags().Changed("endpoint") || configFile == "" {
				endpointPtr = &endpoint
			}

			// Set environment variable - bindings will handle substitution
			if err := config.SetEndpointOverride(endpointPtr); err != nil {
				return err
			}

			// Create manager and start - all validation happens in bindings
			mgr := manager.NewManager(appConfig.CommonOpts.Logger, configMgr)
			return mgr.Start(c.Context())
		},
	}

	startCmd.Flags().StringVarP(&configFile, "config", "c", "",
		"Path to YAML configuration file (production SLIM format)")
	startCmd.Flags().StringVar(&endpoint, "endpoint", "",
		"Server endpoint (sets SLIM_ENDPOINT environment variable)")

	return startCmd
}
