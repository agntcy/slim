// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"context"
	"log"
	"time"

	slim_bindings "github.com/agntcy/slim/bindings/generated/slim_bindings"
	pb "github.com/agntcy/slim/bindings/go/examples/slimrpc/simple/types"
)

func main() {
	// Initialize SLIM with defaults
	slim_bindings.InitializeWithDefaults()

	service := slim_bindings.GetGlobalService()

	// Create local and remote names
	localName := slim_bindings.NewName("agntcy", "grpc", "client")
	remoteName := slim_bindings.NewName("agntcy", "grpc", "server")

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

	// Create channel
	channel := slim_bindings.ChannelNewWithConnection(app, remoteName, &connId)

	// Create client
	client := pb.NewTestClient(channel)

	// Call method
	ctx := context.Background()
	
	request := &pb.ExampleRequest{
		ExampleInteger: 1,
		ExampleString:  "hello",
	}

	response, err := client.ExampleUnaryUnary(ctx, request)
	if err != nil {
		log.Fatalf("ExampleUnaryUnary failed: %#v", err)
	}

	log.Printf("Response: %+v", response)

	// Give time for messages to be sent
	time.Sleep(1 * time.Second)
}
