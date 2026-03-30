// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"context"
	"flag"
	"fmt"
	"log"
	"strings"
	"sync"
	"time"

	slim_bindings "github.com/agntcy/slim-bindings-go"

	"github.com/agntcy/slim/bindings/go/examples/common"
	pb "github.com/agntcy/slim/bindings/go/examples/slimrpc/simple/types"
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
	server := flag.String("server", common.ServerEndpoint(), "SLIM server endpoint")
	servers := flag.String("servers", "server1,server2", "Comma-separated server instance names")
	flag.Parse()

	slim_bindings.InitializeWithDefaults()

	service := slim_bindings.GetGlobalService()

	localName := slim_bindings.NewName("agntcy", "grpc", "client")
	serverInstances := strings.Split(*servers, ",")
	serverNames := make([]*slim_bindings.Name, len(serverInstances))
	for i, s := range serverInstances {
		serverNames[i] = slim_bindings.NewName("agntcy", "grpc", strings.TrimSpace(s))
	}

	app, err := service.CreateAppWithSecret(localName, common.DefaultSharedSecret)
	if err != nil {
		log.Fatalf("Failed to create app: %v", err)
	}

	clientConfig := slim_bindings.NewInsecureClientConfig(*server)
	connId, err := service.Connect(clientConfig)
	if err != nil {
		log.Fatalf("Failed to connect: %v", err)
	}

	if err := app.Subscribe(app.Name(), &connId); err != nil {
		log.Fatalf("Failed to subscribe: %v", err)
	}

	// Group channel targeting all server instances
	channel, err := slim_bindings.ChannelNewGroupWithConnection(app, serverNames, &connId)
	if err != nil {
		log.Fatalf("Failed to create group channel: %v", err)
	}

	client := pb.NewTestGroupClient(channel)

	fmt.Println("SLIM_RPC_GROUP_CLIENT_STARTED")

	runMulticastUnary(client)
	runMulticastUnaryStream(client)
	runMulticastStreamUnary(client)
	runMulticastStreamStream(client)

	fmt.Println("SLIM_RPC_GROUP_CLIENT_DONE")

	// Give time for messages to be sent
	time.Sleep(1 * time.Second)

	if err := channel.CloseAsync(nil); err != nil {
		log.Printf("Failed to close channel: %v", err)
	}
}
