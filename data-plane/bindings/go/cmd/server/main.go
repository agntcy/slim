// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// SLIM Server - Runs a SLIM network node that clients connect to

package main

import (
	"flag"
	"fmt"
	"log"
	"os"
	"os/signal"
	"syscall"
	"time"

	slim "github.com/agntcy/slim/bindings/generated/slim_service"
)

func main() {
	endpoint := flag.String("endpoint", "0.0.0.0:46357", "Server endpoint (host:port)")
	secret := flag.String("shared-secret", "demo-shared-secret-min-32-chars!!", "Shared secret (min 32 chars)")

	flag.Parse()

	fmt.Println("🚀 SLIM Server")
	fmt.Println("==============")
	fmt.Printf("Endpoint: %s\n", *endpoint)
	fmt.Println()

	// Initialize crypto
	slim.InitializeCrypto()

	// Create server app
	serverName := slim.Name{
		Components: []string{"system", "server", "node"},
		Id:         nil,
	}

	app, err := slim.CreateAppWithSecret(serverName, *secret)
	if err != nil {
		log.Fatalf("Failed to create server app: %v", err)
	}
	defer app.Destroy()

	fmt.Printf("✅ Server app created (ID: %d)\n", app.Id())

	// Start server
	config := slim.ServerConfig{
		Endpoint: *endpoint,
		Tls:      slim.TlsConfig{Insecure: true},
	}

	fmt.Printf("🌐 Starting server on %s...\n", *endpoint)
	fmt.Println("   Waiting for clients to connect...")
	fmt.Println()

	// Run server in goroutine (it blocks)
	serverErr := make(chan error, 1)
	go func() {
		if err := app.RunServer(config); err != nil {
			serverErr <- err
		}
	}()

	// Give server a moment to start
	time.Sleep(100 * time.Millisecond)

	fmt.Println("✅ Server running and listening")
	fmt.Println()
	fmt.Println("📡 Clients can now connect")
	fmt.Println()
	fmt.Println("Press Ctrl+C to stop")

	// Wait for interrupt or error
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)

	select {
	case err := <-serverErr:
		log.Fatalf("Server error: %v", err)
	case sig := <-sigChan:
		fmt.Printf("\n\n📋 Received signal: %v\n", sig)
		fmt.Println("🛑 Shutting down...")
	}
}
