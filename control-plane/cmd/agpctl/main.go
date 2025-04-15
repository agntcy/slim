package main

import (
	"log"
	"os"

	"github.com/spf13/cobra"

	"github.com/agntcy/agp/control-plane/internal/cmd/subscription"
	"github.com/agntcy/agp/control-plane/internal/cmd/version"
	"github.com/agntcy/agp/control-plane/internal/options"
)

func main() {
	opts := options.NewOptions()

	rootCmd := &cobra.Command{
		Use:   "agpctl",
		Short: "AGP control CLI",
	}

	// Register subcommands.
	rootCmd.AddCommand(version.NewVersionCmd(opts))
	rootCmd.AddCommand(subscription.NewSubscribeCmd(opts))
	rootCmd.AddCommand(subscription.NewUnsubscribeCmd(opts))

	if err := rootCmd.Execute(); err != nil {
		log.Fatalf("CLI error: %v", err)
		os.Exit(1)
	}
}
