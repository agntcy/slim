// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// SLIM Server - Runs a SLIM network node that clients connect to

package main

import (
	"flag"
	"fmt"
	"os"
	"os/signal"
	"syscall"
	"time"

	slim "github.com/agntcy/slim/bindings/generated/slim_bindings"
)

func main() {
	endpoint := flag.String("endpoint", "0.0.0.0:46357", "Server endpoint (host:port)")

	flag.Parse()

	fmt.Println("🚀 SLIM Server")
	fmt.Println("==============")
	fmt.Printf("Endpoint: %s\n", *endpoint)
	fmt.Println()

	// Initialize crypto - this will create the global service which we will configure as server
	slim.InitializeWithDefaults()

	// Start server
	config := slim.NewInsecureServerConfig(*endpoint)

	fmt.Printf("🌐 Starting server on %s...\n", *endpoint)
	fmt.Println("   Waiting for clients to connect...")
	fmt.Println()

	// Run server in internal tokio task
	slim.GetGlobalService().RunServerAsync(config)

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

	sig := <-sigChan
	fmt.Printf("\n\n📋 Received signal: %v\n", sig)
	fmt.Println("🛑 Shutting down...")
}
