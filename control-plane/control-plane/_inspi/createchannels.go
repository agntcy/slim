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
	conn, err := grpc.NewClient("localhost:50051", grpc.WithTransportCredentials(insecure.NewCredentials()))
	client := controlplaneApi.NewControlPlaneServiceClient(conn) // Replace nil with actual gRPC client connection

	if err != nil {
		log.Fatalf("did not connect: %v", err)
	}
	defer conn.Close()

	createChannelRequest := &controlplaneApi.CreateChannelRequest{
		Moderators: []string{"moderator1", "moderator2"},
	}
	resp, err := client.CreateChannel(context.Background(), createChannelRequest)
	if err != nil {
		fmt.Printf("send request: %v", err.Error())
	}
	channelId := resp.GetChannelId()
	if resp == nil {
		fmt.Println("\nNo channels found")
		return
	}
	fmt.Printf("Received response: %v\n", channelId)

}
