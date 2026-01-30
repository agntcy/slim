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
		Long: `Start the SLIM instance with full production configuration.

Configuration is loaded using the SLIM bindings' native InitializeFromConfig,
which handles all validation and processing. The bindings support environment 
variable substitution using ${env:VARNAME} syntax.

CLI flags set environment variables that can be referenced in config files:
  --endpoint    → SLIM_ENDPOINT
  --insecure    → SLIM_TLS_INSECURE  
  --tls-cert    → SLIM_TLS_CERT
  --tls-key     → SLIM_TLS_KEY
  
Log level can be controlled via RUST_LOG environment variable.

All configuration validation is performed by the bindings library.`,
		RunE: func(c *cobra.Command, _ []string) error {
			// Create config manager - just manages paths, no validation
			configMgr := config.New(configFile)

			// Set environment variables for any CLI flag overrides
			// These will be substituted by the bindings when it reads the config
			var endpointPtr, certPtr, keyPtr *string
			var insecurePtr *bool

			if c.Flags().Changed("endpoint") || configFile == "" {
				endpointPtr = &endpoint
			}
			if c.Flags().Changed("insecure") {
				insecurePtr = &insecure
			}
			if c.Flags().Changed("tls-cert") {
				certPtr = &tlsCertFile
			}
			if c.Flags().Changed("tls-key") {
				keyPtr = &tlsKeyFile
			}

			// Set environment variables - bindings will handle substitution
			if err := config.SetEnvironmentOverrides(endpointPtr, certPtr, keyPtr, insecurePtr); err != nil {
				return err
			}

			// Create manager and start - all validation happens in bindings
			mgr := manager.NewManager(appConfig.CommonOpts.Logger, configMgr)
			return mgr.Start(c.Context())
		},
	}

	startCmd.Flags().StringVarP(&configFile, "config", "c", "", "Path to YAML configuration file (full SLIM format)")
	startCmd.Flags().StringVar(&endpoint, "endpoint", "", "Override server endpoint (sets SLIM_ENDPOINT)")
	startCmd.Flags().BoolVar(&insecure, "insecure", false, "Override to disable TLS (sets SLIM_TLS_INSECURE)")
	startCmd.Flags().StringVar(&tlsCertFile, "tls-cert", "", "Override TLS certificate path (sets SLIM_TLS_CERT)")
	startCmd.Flags().StringVar(&tlsKeyFile, "tls-key", "", "Override TLS key path (sets SLIM_TLS_KEY)")

	return startCmd
}
