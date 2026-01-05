// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Package main implements a web-based SLIM session client using Go and HTMX.
//
// This application provides a participant-focused interface for:
//   - Receiving session invitations from Session Manager
//   - Accepting or declining invitations
//   - Joining and participating in sessions
//   - Sending and receiving messages
//
// Usage:
//
//	# First start the SLIM server
//	task example:server
//
//	# Start the session manager (creates sessions)
//	task example:sessionmgr
//
//	# Then start the session client (joins sessions)
//	task example:sessionclient LOCAL=org/alice/app
//
//	# Open http://localhost:8082 in your browser
package main

import (
	"context"
	"flag"
	"log"
	"os"
	"os/signal"
	"syscall"

	"github.com/agntcy/slim/bindings/go/examples/internal/config"
	"github.com/agntcy/slim/bindings/go/examples/sessionclient/internal/app"
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
	cfg.HTTPPort = 8082 // Different default port than sessionmgr
	cfg.LocalID = "org/client/app"

	flag.IntVar(&cfg.HTTPPort, "port", cfg.HTTPPort, "HTTP server port")
	flag.StringVar(&cfg.SlimEndpoint, "slim", cfg.SlimEndpoint, "SLIM server endpoint")
	flag.StringVar(&cfg.SharedSecret, "secret", cfg.SharedSecret, "SLIM shared secret")
	flag.StringVar(&cfg.LocalID, "local", cfg.LocalID, "Local SLIM identity (org/namespace/app)")
	flag.IntVar(&cfg.InviteTimeout, "timeout", cfg.InviteTimeout, "Invitation timeout in seconds")
	flag.Parse()

	return cfg
}
