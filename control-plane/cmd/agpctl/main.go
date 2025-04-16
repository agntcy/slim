// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"log"
	"os"

	"github.com/spf13/cobra"

	"github.com/agntcy/agp/control-plane/internal/cmd/configure"
	"github.com/agntcy/agp/control-plane/internal/cmd/version"
	"github.com/agntcy/agp/control-plane/internal/options"
)

func main() {
	opts := options.NewOptions()

	rootCmd := &cobra.Command{
		Use:   "agpctl",
		Short: "AGP control CLI",
	}

	rootCmd.AddCommand(version.NewVersionCmd(opts))
	rootCmd.AddCommand(configure.NewConfigureCmd(opts))

	if err := rootCmd.Execute(); err != nil {
		log.Fatalf("CLI error: %v", err)
		os.Exit(1)
	}
}
