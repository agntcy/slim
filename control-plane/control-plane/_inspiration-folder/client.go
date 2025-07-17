package main

import (
	"context"
	"fmt"
	"log"
	"net"
	"strconv"
	"strings"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	grpcapi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/protobuf/types/known/wrapperspb"
)

func main() {
	opts := grpc.WithTransportCredentials(insecure.NewCredentials())
	connection, err := grpc.NewClient("localhost:50051", opts)
	if err != nil {
		log.Fatalf("failed to listen: %v", err)

	}
	defer connection.Close()
	client := grpcapi.NewControlPlaneServiceClient(connection)
	ctx := context.TODO()

	organization, namespace, agentType, agentID, err := parseRoute("org/default/a/0")
	if err != nil {
		log.Fatalf("failed to : %v", err)
	}

	conn, connID, err := parseEndpoint("127.0.0.1:46357")
	if err != nil {
		log.Fatalf("failed to : %v", err)
	}

	subscription := &controllerapi.Subscription{
		Organization: organization,
		Namespace:    namespace,
		AgentType:    agentType,
		ConnectionId: connID,
		AgentId:      wrapperspb.UInt64(agentID),
	}

	controllerConfigCommand := &controllerapi.ConfigurationCommand{
		ConnectionsToCreate:   []*controllerapi.Connection{conn},
		SubscriptionsToSet:    []*controllerapi.Subscription{subscription},
		SubscriptionsToDelete: []*controllerapi.Subscription{},
	}

	returnedMessage, err := client.ModifyConfiguration(ctx, &grpcapi.ConfigurationCommand{
		ConfigurationCommand: controllerConfigCommand,
		NodeId:               "node1",
	})
	if err != nil {
		log.Fatalf("failed to : %v", err)
	}

	fmt.Printf(
		"ACK received for : success=%t\n",
		returnedMessage.Success,
	)
	if len(returnedMessage.Messages) > 0 {
		for i, ackMsg := range returnedMessage.Messages {
			fmt.Printf("    [%d] %s\n", i+1, ackMsg)
		}
	}

}

func parseRoute(route string) (
	organization,
	namespace,
	agentType string,
	agentID uint64,
	err error,
) {
	parts := strings.Split(route, "/")

	if len(parts) != 4 {
		err = fmt.Errorf(
			"invalid route format '%s', expected 'company/namespace/agentname/agentid'",
			route,
		)
		return
	}

	if parts[0] == "" || parts[1] == "" || parts[2] == "" || parts[3] == "" {
		err = fmt.Errorf(
			"invalid route format '%s', expected 'company/namespace/agentname/agentid'",
			route,
		)
		return
	}

	organization = parts[0]
	namespace = parts[1]
	agentType = parts[2]

	agentID, err = strconv.ParseUint(parts[3], 10, 64)
	if err != nil {
		err = fmt.Errorf("invalid agent ID %s", parts[3])
		return
	}

	return
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
