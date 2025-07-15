package main

import (
	"context"
	"fmt"
	"log"
	"net"
	"strconv"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneapi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

func main() {
	opts := grpc.WithTransportCredentials(insecure.NewCredentials())
	connection, err := grpc.NewClient("localhost:50051", opts)
	if err != nil {
		log.Fatalf("failed to listen: %v", err)

	}
	defer connection.Close()
	client := controlplaneapi.NewControlPlaneServiceClient(connection)
	ctx := context.TODO()

	conn, _, err := parseEndpoint("127.0.0.1:46357")
	if err != nil {
		log.Fatalf("failed to : %v", err)
	}

	createConnectionResponse, err := client.CreateConnection(ctx, &controlplaneapi.CreateConnectionRequest{
		NodeId:     "node1",
		Connection: conn,
	})
	if err != nil {
		log.Fatalf("failed to : %v", err)
	}
	fmt.Printf("Received create connection response: %v %v\n", createConnectionResponse.Success, createConnectionResponse.ConnectionId)
}

func parseEndpoint(endpoint string) (*controllerapi.Connection, string, error) {
	host, portStr, err := net.SplitHostPort(endpoint)
	if err != nil {
		return nil, "", fmt.Errorf(
			"cannot split endpoint '%s' into host:port: %w",
			endpoint,
			err,
		)
	}

	if host == "" {
		return nil, "", fmt.Errorf(
			"invalid endpoint format '%s': host part is missing",
			endpoint,
		)
	}

	port, err := strconv.ParseInt(portStr, 10, 32)
	if err != nil {
		return nil, "", fmt.Errorf(
			"invalid port '%s' in endpoint '%s': %w",
			portStr,
			endpoint,
			err,
		)
	}
	if port <= 0 || port > 65535 {
		return nil, "", fmt.Errorf(
			"port number '%d' in endpoint '%s' out of range (1-65535)",
			port,
			endpoint,
		)
	}

	connID := endpoint
	conn := &controllerapi.Connection{
		ConnectionId:  connID,
		RemoteAddress: host,
		RemotePort:    int32(port),
	}

	return conn, connID, nil
}
