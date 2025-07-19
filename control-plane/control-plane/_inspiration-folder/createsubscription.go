package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"os"
	"strconv"
	"strings"

	"github.com/agntcy/slim/control-plane/common/controller"
	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"

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
	client := controlplaneApi.NewControlPlaneServiceClient(connection)
	ctx := context.TODO()

	organization, namespace, agentType, agentID, err := parseRoute("org/default/a/0")
	if err != nil {
		log.Fatalf("failed to : %v", err)
	}

	subscription := &controllerapi.Subscription{
		Organization: organization,
		Namespace:    namespace,
		AgentType:    agentType,
		ConnectionId: "a81bc81b-dead-4e5d-abff-90865d1e13b1",
		AgentId:      wrapperspb.UInt64(agentID),
	}

	createSubscriptionResponse, err := client.CreateSubscription(ctx, &controlplaneApi.CreateSubscriptionRequest{
		NodeId:       "node1",
		Subscription: subscription,
	})
	if err != nil {
		log.Fatalf("failed to : %v", err)
	}
	fmt.Printf("Received create subsription response: %v %v\n", createSubscriptionResponse.Success, createSubscriptionResponse.SubscriptionId)
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

func parseConfigFile(configFile string) (*controllerapi.Connection, error) {
	if configFile == "" {
		return nil, fmt.Errorf("config file path cannot be empty")
	}
	if !strings.HasSuffix(configFile, ".json") {
		return nil, fmt.Errorf("config file '%s' must be a JSON file", configFile)
	}

	// Read the file content as a string
	data, err := os.ReadFile(configFile)
	if err != nil {
		return nil, fmt.Errorf("failed to read config file: %w", err)
	}
	// validate the json data against the ConfigClient Schema
	if !controller.Validate(data) {
		return nil, fmt.Errorf("failed to validate config data")
	}

	configData := string(data)

	// Parse the JSON and extract the endpoint value
	var jsonObj map[string]interface{}
	if err := json.Unmarshal(data, &jsonObj); err != nil {
		return nil, fmt.Errorf("invalid JSON in config file: %w", err)
	}
	endpoint, ok := jsonObj["endpoint"].(string)
	if !ok || endpoint == "" {
		return nil, fmt.Errorf("'endpoint' key not found in config data")
	}

	conn := &controllerapi.Connection{
		ConnectionId: endpoint,
		ConfigData:   configData,
	}

	return conn, nil
}
