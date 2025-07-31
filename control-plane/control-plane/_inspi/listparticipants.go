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

	listParticipantsRequest := &controlplaneApi.ListParticipantsRequest{
		ChannelId: "moderator1-wJcF4BhQbxc4N0icik", // Replace with actual channel ID to delete
	}
	resp, err := client.ListParticipants(context.Background(), listParticipantsRequest)
	if err != nil {
		fmt.Printf("send request: %v", err.Error())
	}

	participantIDs := resp.GetParticipantId()
	if resp == nil {
		fmt.Println("No channels found")
		return
	}
	fmt.Printf("Received response: %v\n", participantIDs)
}
