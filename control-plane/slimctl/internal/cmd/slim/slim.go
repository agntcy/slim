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
		Long:  `Start the SLIM instance with full production-like configuration`,
		RunE: func(c *cobra.Command, _ []string) error {
			// Load full config or use defaults
			var fullConfig *config.FullConfig
			var err error

			if configFile != "" {
				// Load from YAML file
				fullConfig, err = config.LoadFullConfig(configFile)
				if err != nil {
					return err
				}
			} else {
				// Use default configuration
				fullConfig = config.DefaultFullConfig()
			}

			// Apply CLI flag overrides (only if explicitly set)
			var insecurePtr *bool
			if c.Flags().Changed("insecure") {
				insecurePtr = &insecure
			}
			fullConfig.MergeFlags(&endpoint, insecurePtr, &tlsCertFile, &tlsKeyFile)

			// Validate
			if err := fullConfig.Validate(); err != nil {
				return err
			}

			// Create manager with configuration
			mgr := manager.NewManagerWithFullConfig(appConfig.CommonOpts.Logger, fullConfig)
			return mgr.Start(c.Context())
		},
	}

	startCmd.Flags().StringVarP(&configFile, "config", "c", "", "Path to YAML configuration file (full SLIM format)")
	startCmd.Flags().StringVar(&endpoint, "endpoint", "", "Override server endpoint (e.g., 127.0.0.1:8080)")
	startCmd.Flags().BoolVar(&insecure, "insecure", false, "Override to disable TLS (insecure mode)")
	startCmd.Flags().StringVar(&tlsCertFile, "tls-cert", "", "Override TLS certificate file path")
	startCmd.Flags().StringVar(&tlsKeyFile, "tls-key", "", "Override TLS key file path")

	return startCmd
}
