// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/knadh/koanf"
	"github.com/knadh/koanf/parsers/yaml"
	"github.com/knadh/koanf/providers/confmap"
	"github.com/knadh/koanf/providers/env"
	"github.com/knadh/koanf/providers/file"
	"github.com/knadh/koanf/providers/posflag"
	"github.com/spf13/afero"
	"github.com/spf13/cobra"
	"github.com/spf13/pflag"
	"go.uber.org/zap"

	"github.com/agntcy/slim/control-plane/slimctl/internal/cfg"
	"github.com/agntcy/slim/control-plane/slimctl/internal/cmd"
	"github.com/agntcy/slim/control-plane/slimctl/internal/cmd/config"
	"github.com/agntcy/slim/control-plane/slimctl/internal/cmd/controller"
	"github.com/agntcy/slim/control-plane/slimctl/internal/cmd/node"
	"github.com/agntcy/slim/control-plane/slimctl/internal/cmd/slim"
	"github.com/agntcy/slim/control-plane/slimctl/internal/cmd/version"
)

var k = koanf.New(".")

func initConfig(conf *cfg.ConfigData, flagSet *pflag.FlagSet) error {
	// defaults
	defaults := map[string]interface{}{
		"common_opts.basic_auth_creds": "",
		"common_opts.server":           "localhost:50051",
		"common_opts.timeout":          "15s",
		"common_opts.tls_insecure":     true,
		"common_opts.tls_ca_file":      "",
		"common_opts.tls_cert_file":    "",
		"common_opts.tls_key_file":     "",
	}
	if err := k.Load(confmap.Provider(defaults, "."), nil); err != nil {
		return fmt.Errorf("error loading defaults: %w", err)
	}

	home, _ := os.UserHomeDir()
	paths := []string{
		filepath.Join(home, ".config", "slimctl", "config.yaml"),
		"config.yaml",
	}
	for _, p := range paths {
		if exists, _ := afero.Exists(conf.Fs, p); exists {
			if err := k.Load(file.Provider(p), yaml.Parser()); err != nil {
				return fmt.Errorf("error reading config %s: %w", p, err)
			}
			break
		}
	}

	if flagSet != nil {
		if err := k.Load(posflag.ProviderWithValue(flagSet, ".", k, cfg.MapFlagToConfigFunc()), nil); err != nil {
			return fmt.Errorf("error loading flags: %w", err)
		}
	}

	envOpts := env.Provider("SLIMCTL_", ".", func(s string) string {
		return strings.ReplaceAll(
			strings.ToLower(strings.TrimPrefix(s, "SLIMCTL_")),
			"_",
			".",
		)
	})
	if err := k.Load(envOpts, nil); err != nil {
		return fmt.Errorf("error loading env: %w", err)
	}

	conf.AppConfig.CommonOpts.BasicAuthCredentials = k.String("common_opts.basic_auth_creds")
	conf.AppConfig.CommonOpts.Server = k.String("common_opts.server")
	conf.AppConfig.CommonOpts.Timeout = k.Duration("common_opts.timeout")
	conf.AppConfig.CommonOpts.TLSInsecure = k.Bool("common_opts.tls_insecure")
	conf.AppConfig.CommonOpts.TLSCAFile = k.String("common_opts.tls_ca_file")
	conf.AppConfig.CommonOpts.TLSCertFile = k.String("common_opts.tls_cert_file")
	conf.AppConfig.CommonOpts.TLSKeyFile = k.String("common_opts.tls_key_file")

	return nil
}

func main() {
	var cmdFs = afero.NewOsFs()
	conf := cfg.NewConfigData(cmdFs)

	rootCmd := &cobra.Command{
		Use:   "slimctl",
		Short: "SLIM control CLI",
		PersistentPreRunE: func(cmd *cobra.Command, _ []string) error {
			if err := initConfig(conf, cmd.Root().PersistentFlags()); err != nil {
				return err
			}
			// Initialize a default logger if none is provided to avoid nil dereference in commands.
			if conf.AppConfig.CommonOpts.Logger == nil {
				l, err := zap.NewProduction()
				if err != nil {
					return fmt.Errorf("failed to initialize logger: %w", err)
				}
				conf.AppConfig.CommonOpts.Logger = l
			}
			return nil
		},
	}
	rootCmd.PersistentFlags().StringP(
		"basic_auth_creds",
		"b",
		k.String("common_opts.basic_auth_creds"),
		"Basic auth credentials for authentication in username:password format",
	)

	rootCmd.PersistentFlags().StringP(
		"server",
		"s",
		k.String("common_opts.server"),
		"SLIM gRPC control API endpoint (host:port)",
	)

	rootCmd.PersistentFlags().Duration(
		"timeout",
		15*time.Second,
		"gRPC request timeout (e.g. 5s, 1m)",
	)

	rootCmd.PersistentFlags().Bool(
		"tls.insecure",
		true,
		"skip TLS certificate verification",
	)

	rootCmd.PersistentFlags().String(
		"tls.ca_file",
		k.String("common_opts.tls_ca_file"),
		"path to TLS CA certificate",
	)

	rootCmd.PersistentFlags().String(
		"tls.cert_file",
		k.String("common_opts.tls_cert_file"),
		"path to client TLS certificate",
	)

	rootCmd.PersistentFlags().String(
		"tls.key_file",
		k.String("common_opts.tls_key_file"),
		"path to client TLS key",
	)

	// add the command groups
	rootCmd.AddGroup(
		&cobra.Group{ID: cmd.GroupSlimctl, Title: "Commands for SLIM CLI configuration & version info"},
		&cobra.Group{ID: cmd.GroupNode, Title: "Commands to interact with SLIM nodes"},
		&cobra.Group{ID: cmd.GroupController, Title: "Commands to interact with the SLIM Control Plane"},
		&cobra.Group{ID: cmd.GroupSlim, Title: "Commands for managing a local SLIM instance"},
	)

	// add the version command
	rootCmd.AddCommand(
		version.NewVersionCmd(conf.AppConfig.CommonOpts),
		config.NewConfigCmd(conf),
	)

	// add the controller command tree
	rootCmd.AddCommand(controller.NewControllerCmd(conf))

	// add the node command tree
	rootCmd.AddCommand(node.NewNodeCmd(conf.AppConfig.CommonOpts))

	// add the slim command tree
	rootCmd.AddCommand(slim.NewSlimCmd(rootCmd.Context(), conf.AppConfig))

	if err := rootCmd.Execute(); err != nil {
		fmt.Fprintf(os.Stderr, "CLI error: %v", err)
		os.Exit(1)
	}
}
