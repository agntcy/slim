// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/spf13/cobra"
	"github.com/spf13/viper"

	"github.com/agntcy/agp/control-plane/agpctl/internal/cmd/route"
	"github.com/agntcy/agp/control-plane/agpctl/internal/cmd/version"
	"github.com/agntcy/agp/control-plane/agpctl/internal/options"
)

func initConfig(opts *options.CommonOptions) {
	home, _ := os.UserHomeDir()
	viper.AddConfigPath(filepath.Join(home, ".agpctl"))
	viper.AddConfigPath(".")
	viper.SetConfigName("config")
	viper.SetConfigType("yaml")

	viper.SetEnvPrefix("AGPCTL")
	viper.AutomaticEnv()

	// set defaults
	viper.SetDefault("server", "localhost:46357")
	viper.SetDefault("timeout", "5s")
	viper.SetDefault("tls.insecure", true)

	if err := viper.ReadInConfig(); err != nil {
		if _, ok := err.(viper.ConfigFileNotFoundError); !ok {
			fmt.Fprintf(os.Stderr, "Error reading config file: %v\n", err)
			os.Exit(1)
		}
	}

	opts.Server = viper.GetString("server")
	opts.Timeout = viper.GetDuration("timeout")
	opts.TLSInsecure = viper.GetBool("tls.insecure")
	opts.TLSCAFile = viper.GetString("tls.ca_file")
	opts.TLSCertFile = viper.GetString("tls.cert_file")
	opts.TLSKeyFile = viper.GetString("tls.key_file")
}

func main() {
	opts := options.NewOptions()
	rootCmd := &cobra.Command{
		Use:   "agpctl",
		Short: "AGP control CLI",
		PersistentPreRunE: func(cmd *cobra.Command, args []string) error {
			initConfig(opts)
			return nil
		},
	}

	rootCmd.PersistentFlags().StringP("server", "s", "", "gateway gRPC control API endpoint (host:port)")
	_ = viper.BindPFlag("server", rootCmd.PersistentFlags().Lookup("server"))

	rootCmd.PersistentFlags().Duration("timeout", 0, "gRPC request timeout (e.g. 5s, 1m)")
	_ = viper.BindPFlag("timeout", rootCmd.PersistentFlags().Lookup("timeout"))

	rootCmd.PersistentFlags().Bool("tls.insecure", false, "skip TLS certificate verification")
	_ = viper.BindPFlag("tls.insecure", rootCmd.PersistentFlags().Lookup("tls.insecure"))

	rootCmd.PersistentFlags().String("tls.ca_file", "", "path to TLS CA certificate")
	_ = viper.BindPFlag("tls.ca_file", rootCmd.PersistentFlags().Lookup("tls.ca_file"))

	rootCmd.PersistentFlags().String("tls.cert_file", "", "path to client TLS certificate")
	_ = viper.BindPFlag("tls.cert_file", rootCmd.PersistentFlags().Lookup("tls.cert_file"))

	rootCmd.PersistentFlags().String("tls.key_file", "", "path to client TLS key")
	_ = viper.BindPFlag("tls.key_file", rootCmd.PersistentFlags().Lookup("tls.key_file"))

	rootCmd.AddCommand(route.NewRouteCmd(opts))
	rootCmd.AddCommand(version.NewVersionCmd(opts))

	if err := rootCmd.Execute(); err != nil {
		fmt.Fprintf(os.Stderr, "CLI error: %v", err)
		fmt.Println(opts)
		os.Exit(1)
	}
}
