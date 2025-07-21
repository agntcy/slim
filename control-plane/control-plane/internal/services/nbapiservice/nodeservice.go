package nbapiservice

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/agntcy/slim/control-plane/common/controlplane"
	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
)

type nodeService struct {
	dbService db.DataAccess
}

func NewNodeService(dbService db.DataAccess) *nodeService {
	return &nodeService{
		dbService: dbService,
	}
}

func (s *nodeService) ListNodes(context.Context, *controlplaneApi.NodeListRequest) (*controlplaneApi.NodeListResponse, error) {
	storedNodes, err := s.dbService.ListNodes()
	if err != nil {
		return nil, err
	}
	nodeEntries := make([]*controlplaneApi.NodeEntry, 0, len(storedNodes))
	for _, node := range storedNodes {
		nodeEntry := &controlplaneApi.NodeEntry{
			Id:   node.ID,
			Host: node.Host,
			Port: uint32(node.Port),
		}
		nodeEntries = append(nodeEntries, nodeEntry)
	}
	nodeListresponse := &controlplaneApi.NodeListResponse{
		Entries: nodeEntries,
	}
	return nodeListresponse, nil
}

func (s *nodeService) GetNodeByID(nodeID string) (*controlplaneApi.NodeEntry, error) {
	storedNode, err := s.dbService.GetNode(nodeID)
	if err != nil {
		return nil, err
	}
	nodeEntry := &controlplaneApi.NodeEntry{
		Id:   storedNode.ID,
		Host: storedNode.Host,
		Port: uint32(storedNode.Port),
	}
	return nodeEntry, nil
}

func (s *nodeService) SaveConnection(nodeEntry *controlplaneApi.NodeEntry, connection *controllerapi.Connection) (string, error) {
	// Check if the node exists
	if nodeEntry == nil {
		return "", fmt.Errorf("node entry is required")
	}

	if nodeEntry.Id == "" {
		return "", fmt.Errorf("node ID is required")
	}

	_, err := s.dbService.GetNode(nodeEntry.Id)
	if err != nil {
		return "", fmt.Errorf("node with ID %s not found: %v", nodeEntry.Id, err)
	}

	connectionEntry := db.Connection{
		ID: connection.ConnectionId,
		NodeID:     nodeEntry.Id,
		ConfigData: connection.ConfigData,
	}

	return s.dbService.SaveConnection(connectionEntry)
}

func (s *nodeService) GetConnectionDetails(nodeID string, connectionID string) (string, error) {
	connection, err := s.dbService.GetConnection(connectionID)
	if err != nil {
		return "", fmt.Errorf("failed to get connection details: %v", err)
	}
	if connection.NodeID != nodeID {
		return "", fmt.Errorf("connection with ID %s does not belong to node %s", connectionID, nodeID)
	}

	// Retrive endpoint details from the connection's config data

	// Parse the JSON config data
	var config map[string]interface{}
	if err := json.Unmarshal([]byte(connection.ConfigData), &config); err != nil {
		return "", fmt.Errorf("failed to parse config data: %v", err)
	}

	// Extract the endpoint value
	endpoint, exists := config["endpoint"]
	if !exists {
		return "", fmt.Errorf("endpoint not found in config data")
	}

	endpointStr, ok := endpoint.(string)
	if !ok {
		return "", fmt.Errorf("endpoint is not a string")
	}

	return endpointStr, nil
}

func (s *nodeService) SaveSubscription(nodeID string, subscription *controllerapi.Subscription) (string, error) {
	// Check if the node exists
	_, err := s.dbService.GetNode(nodeID)
	if err != nil {
		return "", fmt.Errorf("node with ID %s not found: %v", nodeID, err)
	}

	// Check if the connection exists
	if subscription.ConnectionId == "" {
		return "", fmt.Errorf("connection ID is required")
	}
	_, err = s.dbService.GetConnection(subscription.ConnectionId)
	if err != nil {
		return "", fmt.Errorf("connection with ID %s not found: %v", subscription.ConnectionId, err)
	}

	subscriptionEntry := db.Subscription{
		ID:           controlplane.GetSubscriptionID(subscription),
		NodeID:       nodeID,
		ConnectionID: subscription.ConnectionId,
		Organization: subscription.Organization,
		Namespace:    subscription.Namespace,
		AgentType:    subscription.AgentType,
		AgentID:      subscription.AgentId,
	}

	return s.dbService.SaveSubscription(subscriptionEntry)
}

func (s *nodeService) GetSubscription(nodeID string, subscriptionId string) (*controllerapi.Subscription, error) {
	subscription, err := s.dbService.GetSubscription(subscriptionId)
	if err != nil {
		return nil, fmt.Errorf("failed to get subscription: %v", err)
	}

	if subscription.NodeID != nodeID {
		return nil, fmt.Errorf("subscription with ID %s does not belong to node %s", subscriptionId, nodeID)
	}

	return &controllerapi.Subscription{
		ConnectionId: subscription.ConnectionID,
		Organization: subscription.Organization,
		Namespace:    subscription.Namespace,
		AgentType:    subscription.AgentType,
		AgentId:      subscription.AgentID,
	}, nil
}
