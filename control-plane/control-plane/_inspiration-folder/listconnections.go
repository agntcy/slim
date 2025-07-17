package main

import (
	"context"
	"fmt"
	"log"

	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"

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
	client := controlplaneApi.NewControlPlaneServiceClient(connection)
	ctx := context.TODO()

	connectionListResponse, err := client.ListConnections(ctx, &controlplaneApi.Node{
		Id: "node1",
	})
	if err != nil {
		log.Fatalf("failed to : %v", err)
	}
	fmt.Printf("Received connection list response: %v\n", len(connectionListResponse.Entries))
	for _, entry := range connectionListResponse.Entries {
		fmt.Printf("Connection ID: %v, Connection type: %v, ConfigData %v\n", entry.Id, entry.ConnectionType, entry.ConfigData)
	}
	fmt.Printf("Received connection list response: %v\n", connectionListResponse)

}
