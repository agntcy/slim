package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"os"
	"strings"

	"github.com/agntcy/slim/control-plane/common/controller"
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

	// conn, _, err := parseEndpoint("127.0.0.1:46357")
	// if err != nil {
	// 	log.Fatalf("failed to : %v", err)
	// }

	conn, err := parseConfigFile("/Users/dominik/repos/src/github.com/agntcy/slim/tests/integration/testdata/client-a-config-data.json")
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

// func parseEndpoint(endpoint string) (*controllerapi.Connection, string, error) {
// 	host, portStr, err := net.SplitHostPort(endpoint)
// 	if err != nil {
// 		return nil, "", fmt.Errorf(
// 			"cannot split endpoint '%s' into host:port: %w",
// 			endpoint,
// 			err,
// 		)
// 	}

// 	if host == "" {
// 		return nil, "", fmt.Errorf(
// 			"invalid endpoint format '%s': host part is missing",
// 			endpoint,
// 		)
// 	}

// 	port, err := strconv.ParseInt(portStr, 10, 32)
// 	if err != nil {
// 		return nil, "", fmt.Errorf(
// 			"invalid port '%s' in endpoint '%s': %w",
// 			portStr,
// 			endpoint,
// 			err,
// 		)
// 	}
// 	if port <= 0 || port > 65535 {
// 		return nil, "", fmt.Errorf(
// 			"port number '%d' in endpoint '%s' out of range (1-65535)",
// 			port,
// 			endpoint,
// 		)
// 	}

// 	connID := endpoint
// 	conn := &controllerapi.Connection{
// 		ConnectionId:  connID,
// 		RemoteAddress: host,
// 		RemotePort:    int32(port),
// 	}

// 	return conn, connID, nil
// }

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
