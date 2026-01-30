// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"context"
	"fmt"
	"log"
	"os"
	"os/signal"
	"syscall"

	slim_bindings "github.com/agntcy/slim/bindings/generated/slim_bindings"
	pb "github.com/agntcy/slim/bindings/go/examples/slimrpc/simple/types"
)

type TestServiceImpl struct {
	pb.UnimplementedTestServer
}

func (s *TestServiceImpl) ExampleUnaryUnary(ctx context.Context, req *pb.ExampleRequest) (*pb.ExampleResponse, error) {
	log.Printf("Received unary-unary request: %+v", req)
	return &pb.ExampleResponse{
		ExampleInteger: 1,
		ExampleString:  "Hello, World!",
	}, nil
}

func main() {
	// Initialize SLIM with defaults
	slim_bindings.InitializeWithDefaults()

	service := slim_bindings.GetGlobalService()

	// Create local name
	localName := slim_bindings.NewName("agntcy", "grpc", "server")

	// Create app with shared secret
	app, err := service.CreateAppWithSecret(localName, "my_shared_secret_for_testing_purposes_only")
	if err != nil {
		log.Fatalf("Failed to create app: %v", err)
	}

	// Connect to SLIM
	clientConfig := slim_bindings.NewInsecureClientConfig("http://localhost:46357")
	connId, err := service.Connect(clientConfig)
	if err != nil {
		log.Fatalf("Failed to connect: %v", err)
	}

	// Subscribe to local name
	if err := app.Subscribe(localName, &connId); err != nil {
		log.Fatalf("Failed to subscribe: %v", err)
	}

	// Create server
	server := slim_bindings.ServerNewWithConnection(app, localName, &connId)

	// Register service
	pb.RegisterTestServer(server, &TestServiceImpl{})

	// Set up signal handling
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, os.Interrupt, syscall.SIGTERM)

	// Start server in a goroutine
	go func() {
		log.Println("Server starting...")
		if err := server.Serve(); err != nil {
			log.Printf("Server error: %v", err)
		}
	}()

	// Wait for interrupt signal
	<-sigChan
	fmt.Println("\nServer interrupted by user.")
}
