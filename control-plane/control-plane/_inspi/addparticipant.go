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

	addParticipantRequest := &controlplaneApi.AddParticipantRequest{
		ParticipantId: "participant1",                  // Replace with actual participant ID to add
		ChannelId:     "moderator1-wJcF4BhQbxc4N0icik", // Replace with actual channel ID to delete
	}
	ack, err := client.AddParticipant(context.Background(), addParticipantRequest)
	if err != nil {
		fmt.Printf("send request: %v", err.Error())
	}

	fmt.Printf(
		"ACK received for %s: success=%t\n",
		ack.OriginalMessageId,
		ack.Success,
	)

}
