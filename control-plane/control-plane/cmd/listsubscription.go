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

	subscriptionListResponse, err := client.ListSubscriptions(ctx, &controlplaneApi.Node{
		Id: "node1",
	})
	if err != nil {
		log.Fatalf("failed to : %v", err)
	}
	fmt.Printf("Received connection list response: %v\n", len(subscriptionListResponse.Entries))
	for _, e := range subscriptionListResponse.Entries {
		var localNames, remoteNames []string
		for _, c := range e.LocalConnections {
			localNames = append(localNames,
				fmt.Sprintf("local:%d", c.Id))
		}
		for _, c := range e.RemoteConnections {
			remoteNames = append(remoteNames,
				fmt.Sprintf("remote:%s:%d:%d", c.Ip, c.Port, c.Id))
		}
		fmt.Printf("%s/%s/%s id=%d local=%v remote=%v\n",
			e.Organization, e.Namespace, e.AgentType,
			e.AgentId,
			localNames, remoteNames,
		)
	}
	fmt.Printf("Received connection list response: %v\n", subscriptionListResponse)

}
