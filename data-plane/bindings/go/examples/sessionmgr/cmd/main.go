// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Package main implements a web-based SLIM session manager using Go and HTMX.
//
// This application provides a dashboard for managing SLIM sessions including:
//   - Creating Point-to-Point and Group sessions
//   - Viewing session details (ID, type, destination, participants, messages)
//   - Inviting and removing participants from group sessions
//   - Sending and receiving messages per session
//   - Deleting/discarding sessions
//
// Usage:
//
//	# First start the SLIM server
//	task example:server
//
//	# Then start the session manager
//	task example:sessionmgr
//
//	# Open http://localhost:8081 in your browser
package main

import (
	"context"
	"flag"
	"log"
	"os"
	"os/signal"
	"syscall"

	"github.com/agntcy/slim/bindings/go/examples/internal/config"
	"github.com/agntcy/slim/bindings/go/examples/sessionmgr/internal/app"
)

func main() {
	cfg := parseFlags()

	if err := cfg.Validate(); err != nil {
		log.Fatalf("Invalid configuration: %v", err)
	}

	// Create context with cancellation
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	// Handle shutdown signals
	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM)
	go func() {
		<-sigCh
		log.Println("Shutdown signal received")
		cancel()
	}()

	// Create and run application
	application, err := app.New(cfg)
	if err != nil {
		log.Fatalf("Failed to create application: %v", err)
	}
	defer application.Close()

	if err := application.Run(ctx); err != nil {
		log.Fatalf("Application error: %v", err)
	}
}

func parseFlags() *config.Config {
	cfg := config.NewConfig()
	cfg.LocalID = "org/sessionmgr/app"

	flag.IntVar(&cfg.HTTPPort, "port", cfg.HTTPPort, "HTTP server port")
	flag.StringVar(&cfg.SlimEndpoint, "slim", cfg.SlimEndpoint, "SLIM server endpoint")
	flag.StringVar(&cfg.SharedSecret, "secret", cfg.SharedSecret, "SLIM shared secret")
	flag.StringVar(&cfg.LocalID, "local", cfg.LocalID, "Local SLIM identity (org/namespace/app)")
	flag.Parse()

	return cfg
}
