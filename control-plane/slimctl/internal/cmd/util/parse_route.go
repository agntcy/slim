package util

import (
	"encoding/json"
	"fmt"
	"net/url"
	"os"
	"strconv"
	"strings"

	"github.com/agntcy/slim/control-plane/common/controller"
	grpcapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
)

func ParseRoute(route string) (
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

func ParseEndpoint(endpoint string) (*grpcapi.Connection, string, error) {
	u, err := url.Parse(endpoint)
	if err != nil {
		return nil, "", fmt.Errorf("failed to parse endpoint '%s': %w", endpoint, err)
	}

	scheme := u.Scheme
	if scheme != "http" && scheme != "https" {
		return nil, "", fmt.Errorf("unsupported scheme '%s' in endpoint '%s', must be 'http' or 'https'", scheme, endpoint)
	}

	host := u.Hostname()
	portStr := u.Port()
	if host == "" {
		return nil, "", fmt.Errorf("invalid endpoint format '%s': host part is missing", endpoint)
	}
	if portStr == "" {
		return nil, "", fmt.Errorf("invalid endpoint format '%s': port part is missing", endpoint)
	}
	port, err := strconv.ParseInt(portStr, 10, 32)
	if err != nil {
		return nil, "", fmt.Errorf("invalid port '%s' in endpoint '%s': %w", portStr, endpoint, err)
	}
	if port <= 0 || port > 65535 {
		return nil, "", fmt.Errorf("port number '%d' in endpoint '%s' out of range (1-65535)", port, endpoint)
	}

	conn := &grpcapi.Connection{
		ConnectionId: endpoint,
		ConfigData:   "",
	}

	return conn, endpoint, nil
}

func ParseConfigFile(configFile string) (*grpcapi.Connection, error) {
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

	conn := &grpcapi.Connection{
		ConnectionId: endpoint,
		ConfigData:   configData,
	}

	return conn, nil
}
