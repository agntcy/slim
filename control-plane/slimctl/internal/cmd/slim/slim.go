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
	var insecure bool
	var tlsCertFile string
	var tlsKeyFile string

	startCmd := &cobra.Command{
		Use:   "start",
		Short: "Start the SLIM instance",
		Long:  `Start the SLIM instance`,
		RunE: func(c *cobra.Command, _ []string) error {
			// Load configuration
			var slimConfig *config.SlimConfig
			var err error

			if configFile != "" {
				// Load from YAML file
				slimConfig, err = config.LoadFromFile(configFile)
				if err != nil {
					return err
				}
			} else {
				// Use default configuration
				slimConfig = config.DefaultSlimConfig()
			}

			// Merge CLI flag overrides
			slimConfig.MergeFlags(&endpoint, &insecure, &tlsCertFile, &tlsKeyFile)

			// Create manager with configuration
			mgr := manager.NewManagerWithConfig(appConfig.CommonOpts.Logger, slimConfig)
			return mgr.Start(c.Context())
		},
	}

	startCmd.Flags().StringVarP(&configFile, "config", "c", "", "Path to YAML configuration file")
	startCmd.Flags().StringVar(&endpoint, "endpoint", "", "Endpoint to bind (e.g., 127.0.0.1:8080)")
	startCmd.Flags().BoolVar(&insecure, "insecure", false, "Disable TLS (insecure mode)")
	startCmd.Flags().StringVar(&tlsCertFile, "tls-cert", "", "Path to TLS certificate file")
	startCmd.Flags().StringVar(&tlsKeyFile, "tls-key", "", "Path to TLS key file")

	return startCmd
}
