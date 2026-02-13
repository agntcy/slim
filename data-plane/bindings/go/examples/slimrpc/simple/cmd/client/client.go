// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"context"
	"log"
	"time"

	slim_bindings "github.com/agntcy/slim-bindings-go"
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
	clientConfig := slim_bindings.NewInsecureClientConfig("http://localhost:46358")
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

	// 1. Unary-Unary
	log.Println("=== Unary-Unary ===")
	request := &pb.ExampleRequest{
		ExampleInteger: 1,
		ExampleString:  "hello",
	}

	response, err := client.ExampleUnaryUnary(ctx, request)
	if err != nil {
		log.Fatalf("ExampleUnaryUnary3 failed: %v", err)
	}
	log.Printf("Response: %+v", response)

	// 2. Unary-Stream
	log.Println("\n=== Unary-Stream ===")
	streamClient, err := client.ExampleUnaryStream(ctx, request)
	if err != nil {
		log.Fatalf("ExampleUnaryStream failed: %v", err)
	}

	log.Printf("streamClient: %#v", streamClient)

	for {
		resp, err := streamClient.Recv()
		if err != nil {
			log.Fatalf("Recv failed: %v", err)
		}
		if resp == nil {
			log.Println("Stream ended")
			break
		}
		log.Printf("Stream Response: %+v", resp)
	}

	// 3. Stream-Unary
	log.Println("\n=== Stream-Unary ===")
	streamUnaryClient, err := client.ExampleStreamUnary(ctx)
	if err != nil {
		log.Fatalf("ExampleStreamUnary failed: %v", err)
	}

	for i := int64(0); i < 10; i++ {
		req := &pb.ExampleRequest{
			ExampleInteger: i,
			ExampleString:  "Request " + string(rune('0'+i)),
		}
		if err := streamUnaryClient.Send(req); err != nil {
			log.Fatalf("Send failed: %v", err)
		}
		log.Printf("Sent: %+v", req)
	}

	streamUnaryResp, err := streamUnaryClient.CloseAndRecv()
	if err != nil {
		log.Fatalf("CloseAndRecv failed: %v", err)
	}
	log.Printf("Stream Unary Response: %+v", streamUnaryResp)

	// 4. Stream-Stream
	log.Println("\n=== Stream-Stream ===")
	streamStreamClient, err := client.ExampleStreamStream(ctx)
	if err != nil {
		log.Fatalf("ExampleStreamStream failed: %v", err)
	}

	// Send requests in a goroutine
	go func() {
		for i := int64(0); i < 10; i++ {
			req := &pb.ExampleRequest{
				ExampleInteger: i,
				ExampleString:  "Request " + string(rune('0'+i)),
			}
			if err := streamStreamClient.Send(req); err != nil {
				log.Printf("Send failed: %v", err)
				return
			}
			log.Printf("Sent: %+v", req)
			time.Sleep(100 * time.Millisecond)
		}
		if err := streamStreamClient.CloseSend(); err != nil {
			log.Printf("CloseSend failed: %v", err)
		}
	}()

	// Receive responses
	for {
		resp, err := streamStreamClient.Recv()
		if resp == nil {
			log.Println("Stream Stream completed")
			break
		}
		if err != nil {
			log.Fatalf("Recv failed: %v", err)
		}
		log.Printf("Stream Stream Response: %+v", resp)
	}

	// Give time for messages to be sent
	time.Sleep(1 * time.Second)
}
