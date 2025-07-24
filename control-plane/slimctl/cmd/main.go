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
	"github.com/spf13/cobra"
	"github.com/spf13/pflag"

	"github.com/agntcy/slim/control-plane/common/options"
	controllerCmd "github.com/agntcy/slim/control-plane/slimctl/internal/cmd/controller"
	nodectrlCmd "github.com/agntcy/slim/control-plane/slimctl/internal/cmd/nodecontroller"
	versionCmd "github.com/agntcy/slim/control-plane/slimctl/internal/cmd/version"
)

var k = koanf.New(".")

func initConfig(opts *options.CommonOptions, flagSet *pflag.FlagSet) error {
	// defaults
	defaults := map[string]interface{}{
		"server":        "localhost:46357",
		"timeout":       "5s",
		"tls.insecure":  true,
		"tls.ca_file":   "",
		"tls.cert_file": "",
		"tls.key_file":  "",
	}
	if err := k.Load(confmap.Provider(defaults, "."), nil); err != nil {
		return fmt.Errorf("error loading defaults: %w", err)
	}

	home, _ := os.UserHomeDir()
	paths := []string{
		filepath.Join(home, ".slimctl", "config.yaml"),
		"config.yaml",
	}
	for _, p := range paths {
		if _, err := os.Stat(p); err == nil {
			if err := k.Load(file.Provider(p), yaml.Parser()); err != nil {
				return fmt.Errorf("error reading config %s: %w", p, err)
			}
			break
		}
	}

	if flagSet != nil {
		if err := k.Load(posflag.Provider(flagSet, ".", k), nil); err != nil {
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

	opts.Server = k.String("server")
	opts.Timeout = k.Duration("timeout")
	opts.TLSInsecure = k.Bool("tls.insecure")
	opts.TLSCAFile = k.String("tls.ca_file")
	opts.TLSCertFile = k.String("tls.cert_file")
	opts.TLSKeyFile = k.String("tls.key_file")

	return nil
}

func main() {
	opts := options.NewOptions()

	rootCmd := &cobra.Command{
		Use:   "slimctl",
		Short: "SLIM control CLI",
		PersistentPreRunE: func(cmd *cobra.Command, _ []string) error {
			return initConfig(opts, cmd.Root().PersistentFlags())
		},
	}

	rootCmd.PersistentFlags().StringP(
		"server",
		"s",
		k.String("server"),
		"SLIM gRPC control API endpoint (host:port)",
	)

	rootCmd.PersistentFlags().Duration(
		"timeout",
		5*time.Second,
		"gRPC request timeout (e.g. 5s, 1m)",
	)

	rootCmd.PersistentFlags().Bool(
		"tls.insecure",
		true,
		"skip TLS certificate verification",
	)

	rootCmd.PersistentFlags().String(
		"tls.ca_file",
		"",
		"path to TLS CA certificate",
	)

	rootCmd.PersistentFlags().String(
		"tls.cert_file",
		"",
		"path to client TLS certificate",
	)

	rootCmd.PersistentFlags().String(
		"tls.key_file",
		"",
		"path to client TLS key",
	)

	rootCmd.AddCommand(controllerCmd.NewNodeCmd(opts))
	rootCmd.AddCommand(controllerCmd.NewConnectionCmd(opts))
	rootCmd.AddCommand(controllerCmd.NewRouteCmd(opts))

	rootCmd.AddCommand(nodectrlCmd.NewNodeCmd(opts))
	rootCmd.AddCommand(versionCmd.NewVersionCmd(opts))

	if err := rootCmd.Execute(); err != nil {
		fmt.Fprintf(os.Stderr, "CLI error: %v", err)
		os.Exit(1)
	}
}
