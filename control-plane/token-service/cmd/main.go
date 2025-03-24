// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"context"
	"fmt"
	"os"

	"github.com/agntcy/agp/control-plane/internal/options"
	cmd "github.com/agntcy/agp/control-plane/token-service/cmd/commands"
)

func main() {
	// Set up the root cmd.
	opt := options.NewOptions()
	rootCmd, err := cmd.New(opt)
	if err != nil {
		fmt.Print(err)
		os.Exit(1)
	}

	// Execute the command.
	if err := cmd.Execute(context.Background(), rootCmd, opt); err != nil {
		os.Exit(1)
	}

	os.Exit(0)
}
