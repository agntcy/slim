package nbapiservice

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/agntcy/slim/control-plane/common/controlplane"
	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
)

type NodeService struct {
	dbService  db.DataAccess
	cmdHandler nodecontrol.NodeCommandHandler
}

func NewNodeService(dbService db.DataAccess, cmdHandler nodecontrol.NodeCommandHandler) *NodeService {
	return &NodeService{
		dbService:  dbService,
		cmdHandler: cmdHandler,
	}
}

func (s *NodeService) ListNodes(
	context.Context, *controlplaneApi.NodeListRequest,
) (*controlplaneApi.NodeListResponse, error) {
	storedNodes, err := s.dbService.ListNodes()
	if err != nil {
		return nil, err
	}
	nodeEntries := make([]*controlplaneApi.NodeEntry, 0, len(storedNodes))
	for _, node := range storedNodes {
		nodeEntry := &controlplaneApi.NodeEntry{
			Id:   node.ID,
			Host: node.Host,
			Port: node.Port,
		}
		nodeStatus, _ := s.cmdHandler.GetConnectionStatus(node.ID)
		switch nodeStatus {
		case nodecontrol.NodeStatusConnected:
			nodeEntry.Status = controlplaneApi.NodeStatus_CONNECTED
		case nodecontrol.NodeStatusNotConnected:
			nodeEntry.Status = controlplaneApi.NodeStatus_NOT_CONNECTED
		}
		nodeEntries = append(nodeEntries, nodeEntry)
	}
	nodeListresponse := &controlplaneApi.NodeListResponse{
		Entries: nodeEntries,
	}
	return nodeListresponse, nil
}

func (s *NodeService) GetNodeByID(nodeID string) (*controlplaneApi.NodeEntry, error) {
	storedNode, err := s.dbService.GetNode(nodeID)
	if err != nil {
		return nil, err
	}
	nodeEntry := &controlplaneApi.NodeEntry{
		Id:   storedNode.ID,
		Host: storedNode.Host,
		Port: storedNode.Port,
	}
	return nodeEntry, nil
}

func (s *NodeService) SaveConnection(
	nodeEntry *controlplaneApi.NodeEntry, connection *controllerapi.Connection,
) (string, error) {
	// Check if the node exists
	if nodeEntry == nil {
		return "", fmt.Errorf("node entry is required")
	}

	if nodeEntry.Id == "" {
		return "", fmt.Errorf("node ID is required")
	}

	_, err := s.dbService.GetNode(nodeEntry.Id)
	if err != nil {
		return "", fmt.Errorf("node with ID %s not found: %w", nodeEntry.Id, err)
	}

	connectionEntry := db.Connection{
		ID:         connection.ConnectionId,
		NodeID:     nodeEntry.Id,
		ConfigData: connection.ConfigData,
	}

	return s.dbService.SaveConnection(connectionEntry)
}

func (s *NodeService) GetConnectionDetails(nodeID string, connectionID string) (string, error) {
	connection, err := s.dbService.GetConnection(connectionID)
	if err != nil {
		return "", fmt.Errorf("failed to get connection details: %w", err)
	}
	if connection.NodeID != nodeID {
		return "", fmt.Errorf("connection with ID %s does not belong to node %s", connectionID, nodeID)
	}

	// Retrieve endpoint details from the connection's config data

	// Parse the JSON config data
	var config map[string]interface{}
	if err := json.Unmarshal([]byte(connection.ConfigData), &config); err != nil {
		return "", fmt.Errorf("failed to parse config data: %w", err)
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

func (s *NodeService) SaveSubscription(nodeID string, subscription *controllerapi.Subscription) (string, error) {
	// Check if the node exists
	_, err := s.dbService.GetNode(nodeID)
	if err != nil {
		return "", fmt.Errorf("node with ID %s not found: %w", nodeID, err)
	}

	// Check if the connection exists
	if subscription.ConnectionId == "" {
		return "", fmt.Errorf("connection ID is required")
	}
	_, err = s.dbService.GetConnection(subscription.ConnectionId)
	if err != nil {
		return "", fmt.Errorf("connection with ID %s not found: %w", subscription.ConnectionId, err)
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

func (s *NodeService) GetSubscription(nodeID string, subscriptionID string) (*controllerapi.Subscription, error) {
	subscription, err := s.dbService.GetSubscription(subscriptionID)
	if err != nil {
		return nil, fmt.Errorf("failed to get subscription: %w", err)
	}

	if subscription.NodeID != nodeID {
		return nil, fmt.Errorf("subscription with ID %s does not belong to node %s", subscriptionID, nodeID)
	}

	return &controllerapi.Subscription{
		ConnectionId: subscription.ConnectionID,
		Organization: subscription.Organization,
		Namespace:    subscription.Namespace,
		AgentType:    subscription.AgentType,
		AgentId:      subscription.AgentID,
	}, nil
}
