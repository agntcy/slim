// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"log"
	"os"

	"github.com/spf13/cobra"

	"github.com/agntcy/agp/control-plane/internal/cmd/route"
	"github.com/agntcy/agp/control-plane/internal/cmd/version"
	"github.com/agntcy/agp/control-plane/internal/options"
)

func main() {
	opts := options.NewOptions()

	rootCmd := &cobra.Command{
		Use:   "agpctl",
		Short: "AGP control CLI",
	}

	rootCmd.AddCommand(route.NewRouteCmd(opts))
	rootCmd.AddCommand(version.NewVersionCmd(opts))

	if err := rootCmd.Execute(); err != nil {
		log.Fatalf("CLI error: %v", err)
		os.Exit(1)
	}
}
