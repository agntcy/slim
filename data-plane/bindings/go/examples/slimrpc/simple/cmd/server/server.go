// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"context"
	"flag"
	"fmt"
	"log"
	"os"
	"os/signal"
	"syscall"

	slim_bindings "github.com/agntcy/slim-bindings-go"
	"github.com/agntcy/slim-bindings-go/slimrpc"
	"github.com/agntcy/slim/bindings/go/examples/common"
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

	// If you need to return a specific error
	// return nil, slim_bindings.NewRpcErrorRpc(
	// 	   slim_bindings.RpcCodeInvalidArgument,
	// 	   "Invalid argument..",
	// 	   nil,
	// )
	//
	// If you need to return a simple error
	// return nil, fmt.Errorf("error")
	// This will be trated with an "INTERNAL" error code
}

func (s *TestServiceImpl) ExampleUnaryStream(ctx context.Context, req *pb.ExampleRequest, stream slimrpc.RequestStream[*pb.ExampleResponse]) error {
	log.Printf("Received unary-stream request: %+v", req)

	// Generate response stream
	for i := int64(0); i < 5; i++ {
		log.Printf("Sending response %d", i)
		if err := stream.Send(&pb.ExampleResponse{
			ExampleInteger: i,
			ExampleString:  fmt.Sprintf("Response %d", i),
		}); err != nil {
			return err
		}
	}
	log.Println("Finished sending responses")
	return nil
}

func (s *TestServiceImpl) ExampleStreamUnary(ctx context.Context, stream slimrpc.ResponseStream[*pb.ExampleRequest]) (*pb.ExampleResponse, error) {
	log.Println("Received stream-unary request")

	var sum int64
	var receivedStrs []string
	for {
		req, err := stream.Recv()
		if err != nil {
			return nil, err
		}
		if req == nil {
			break
		}
		log.Printf("Received request: %+v", req)
		sum += req.ExampleInteger
		receivedStrs = append(receivedStrs, req.ExampleString)
	}

	log.Printf("Stream ended, received %d messages with sum %d", len(receivedStrs), sum)

	// Return final response
	return &pb.ExampleResponse{
		ExampleInteger: sum,
		ExampleString:  fmt.Sprintf("Received %d messages", len(receivedStrs)),
	}, nil
}

func (s *TestServiceImpl) ExampleStreamStream(ctx context.Context, stream slimrpc.ServerBidiStream[*pb.ExampleRequest, *pb.ExampleResponse]) error {
	log.Println("Received stream-stream request")

	for {
		req, err := stream.Recv()
		if err != nil {
			return err
		}
		if req == nil {
			break
		}
		log.Printf("Echoing back request: %+v", req)
		if err := stream.Send(&pb.ExampleResponse{
			ExampleInteger: req.ExampleInteger * 100,
			ExampleString:  fmt.Sprintf("Echo: %s", req.ExampleString),
		}); err != nil {
			return err
		}
	}
	return nil
}

func main() {
	// instance allows running multiple servers (e.g. server1, server2 for group example)
	instance := flag.String("instance", "server", "Instance name used as the SLIM app name")
	flag.Parse()

	// Initialize SLIM with defaults
	slim_bindings.InitializeWithDefaults()

	service := slim_bindings.GetGlobalService()

	// Create local name
	localName := slim_bindings.NewName("agntcy", "grpc", *instance)

	// Create app with shared secret
	app, err := service.CreateAppWithSecret(localName, common.DefaultSharedSecret)
	if err != nil {
		log.Fatalf("Failed to create app: %v", err)
	}

	// Connect to SLIM
	clientConfig := slim_bindings.NewInsecureClientConfig(common.DefaultServerEndpoint)
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
		log.Printf("Server '%s' starting...", *instance)
		if err := server.Serve(); err != nil {
			log.Printf("Server error: %v", err)
		}
	}()

	// Wait for interrupt signal
	<-sigChan
	fmt.Println("\nServer interrupted by user.")
}
