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
	msg := &controlplaneApi.ListChannelsRequest{}

	conn, err := grpc.NewClient("localhost:50051", grpc.WithTransportCredentials(insecure.NewCredentials()))
	client := controlplaneApi.NewControlPlaneServiceClient(conn) // Replace nil with actual gRPC client connection

	if err != nil {
		log.Fatalf("did not connect: %v", err)
	}
	defer conn.Close()

	resp, err := client.ListChannels(context.Background(), msg)
	if err != nil {
		fmt.Printf("send request: %v\n", err.Error())
	}
	channelIds := resp.GetChannelId()
	if resp == nil {
		fmt.Println("\n No channels found")
		return
	}
	fmt.Printf("\nReceived response: %v\n", channelIds)

}
