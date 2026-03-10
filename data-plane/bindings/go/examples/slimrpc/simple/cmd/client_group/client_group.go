//go:build slim_multicast

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"context"
	"fmt"
	"log"
	"sync"
	"time"

	slim_bindings "github.com/agntcy/slim-bindings-go"

	pb "github.com/agntcy/slim/bindings/go/examples/slimrpc/simple/types"
)

const (
	slimAddr     = "http://localhost:46357"
	sharedSecret = "my_shared_secret_for_testing_purposes_only"
)

func runMulticastUnary(client pb.TestGroupClient) {
	log.Println("=== Multicast Unary-Unary ===")
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	stream, err := client.ExampleUnaryUnary(ctx, &pb.ExampleRequest{ExampleInteger: 1, ExampleString: "hello"})
	if err != nil {
		log.Printf("ExampleUnaryUnary failed: %v", err)
		return
	}
	for {
		item, err := stream.Recv()
		if err != nil {
			log.Printf("Recv error: %v", err)
			return
		}
		if item == nil {
			break
		}
		log.Printf("  [%s] %+v", item.Context, item.Value)
	}
}

func runMulticastUnaryStream(client pb.TestGroupClient) {
	log.Println("\n=== Multicast Unary-Stream ===")
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	stream, err := client.ExampleUnaryStream(ctx, &pb.ExampleRequest{ExampleInteger: 1, ExampleString: "hello"})
	if err != nil {
		log.Printf("ExampleUnaryStream failed: %v", err)
		return
	}
	for {
		item, err := stream.Recv()
		if err != nil {
			log.Printf("Recv error: %v", err)
			return
		}
		if item == nil {
			break
		}
		log.Printf("  [%s] %+v", item.Context, item.Value)
	}
}

func runMulticastStreamUnary(client pb.TestGroupClient) {
	log.Println("\n=== Multicast Stream-Unary ===")
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	stream, err := client.ExampleStreamUnary(ctx)
	if err != nil {
		log.Printf("ExampleStreamUnary failed: %v", err)
		return
	}

	for i := int64(0); i < 3; i++ {
		if err := stream.Send(&pb.ExampleRequest{ExampleInteger: i, ExampleString: fmt.Sprintf("item %d", i)}); err != nil {
			log.Printf("Send failed: %v", err)
			return
		}
	}
	if err := stream.CloseSend(); err != nil {
		log.Printf("CloseSend failed: %v", err)
		return
	}

	for {
		item, err := stream.Recv()
		if err != nil {
			log.Printf("Recv error: %v", err)
			return
		}
		if item == nil {
			break
		}
		log.Printf("  [%s] %+v", item.Context, item.Value)
	}
}

func runMulticastStreamStream(client pb.TestGroupClient) {
	log.Println("\n=== Multicast Stream-Stream ===")
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	stream, err := client.ExampleStreamStream(ctx)
	if err != nil {
		log.Printf("ExampleStreamStream failed: %v", err)
		return
	}

	var wg sync.WaitGroup
	wg.Add(1)
	go func() {
		defer wg.Done()
		for i := int64(0); i < 3; i++ {
			if err := stream.Send(&pb.ExampleRequest{ExampleInteger: i, ExampleString: fmt.Sprintf("item %d", i)}); err != nil {
				log.Printf("Send failed: %v", err)
				return
			}
		}
		if err := stream.CloseSend(); err != nil {
			log.Printf("CloseSend failed: %v", err)
		}
	}()

	for {
		item, err := stream.Recv()
		if err != nil {
			log.Printf("Recv error: %v", err)
			wg.Wait()
			return
		}
		if item == nil {
			wg.Wait()
			break
		}
		log.Printf("  [%s] %+v", item.Context, item.Value)
	}
}

func main() {
	slim_bindings.InitializeWithDefaults()

	service := slim_bindings.GetGlobalService()

	localName := slim_bindings.NewName("agntcy", "grpc", "client")
	serverNames := []*slim_bindings.Name{
		slim_bindings.NewName("agntcy", "grpc", "server1"),
		slim_bindings.NewName("agntcy", "grpc", "server2"),
	}

	app, err := service.CreateAppWithSecret(localName, sharedSecret)
	if err != nil {
		log.Fatalf("Failed to create app: %v", err)
	}

	clientConfig := slim_bindings.NewInsecureClientConfig(slimAddr)
	connId, err := service.Connect(clientConfig)
	if err != nil {
		log.Fatalf("Failed to connect: %v", err)
	}

	if err := app.Subscribe(localName, &connId); err != nil {
		log.Fatalf("Failed to subscribe: %v", err)
	}

	// Group channel targeting all server instances
	channel := slim_bindings.ChannelNewGroupWithConnection(app, serverNames, &connId)

	client := pb.NewTestGroupClient(channel)

	runMulticastUnary(client)
	runMulticastUnaryStream(client)
	runMulticastStreamUnary(client)
	runMulticastStreamStream(client)

	time.Sleep(1 * time.Second)
}
