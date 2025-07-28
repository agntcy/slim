package nbapiservice

import (
	"context"
	"fmt"
	"reflect"

	"github.com/google/uuid"
	"github.com/rs/zerolog"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
)

// NodeRegistrationService handles node registration events by synchronizing
// stored connections and subscriptions from the database to newly registered nodes
type NodeRegistrationService struct {
	dbService          db.DataAccess
	nodeCommandHandler nodecontrol.NodeCommandHandler
}

// NewNodeRegistrationService creates a new instance of NodeRegistrationService
func NewNodeRegistrationService(
	dbService db.DataAccess,
	nodeCommandHandler nodecontrol.NodeCommandHandler,
) *NodeRegistrationService {
	return &NodeRegistrationService{
		dbService:          dbService,
		nodeCommandHandler: nodeCommandHandler,
	}
}

// NodeRegistered implements NodeRegistrationListener interface
// When a node is registered, it fetches all connections and subscriptions from DbService
// and wraps them in a ConfigurationCommand message to send to the node
func (s *NodeRegistrationService) NodeRegistered(ctx context.Context, nodeID string) error {
	zlog := zerolog.Ctx(ctx).With().Str("node_id", nodeID).Logger()

	zlog.Info().Msg("Processing node registration - fetching stored connections and subscriptions")

	// Fetch connections from database
	connections, err := s.dbService.ListConnectionsByNodeID(nodeID)
	if err != nil {
		return fmt.Errorf("failed to fetch connections for node %s: %w", nodeID, err)
	}

	// Fetch subscriptions from database
	subscriptions, err := s.dbService.ListSubscriptionsByNodeID(nodeID)
	if err != nil {
		return fmt.Errorf("failed to fetch subscriptions for node %s: %w", nodeID, err)
	}

	// Convert database connections to controller API format
	var apiConnections []*controllerapi.Connection
	for _, conn := range connections {
		apiConnection := &controllerapi.Connection{
			ConnectionId: conn.ID,
			ConfigData:   conn.ConfigData,
		}
		apiConnections = append(apiConnections, apiConnection)
	}

	// Convert database subscriptions to controller API format
	var apiSubscriptions []*controllerapi.Subscription
	for _, sub := range subscriptions {
		apiSubscription := &controllerapi.Subscription{
			ConnectionId: sub.ConnectionID,
			Organization: sub.Organization,
			Namespace:    sub.Namespace,
			AgentType:    sub.AgentType,
			AgentId:      sub.AgentID,
		}
		apiSubscriptions = append(apiSubscriptions, apiSubscription)
	}

	// Create configuration command with all stored connections and subscriptions
	configCommand := &controllerapi.ConfigurationCommand{
		ConnectionsToCreate:   apiConnections,
		SubscriptionsToSet:    apiSubscriptions,
		SubscriptionsToDelete: []*controllerapi.Subscription{}, // Empty for registration
	}

	// Create control message with configuration command
	messageID := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageID,
		Payload: &controllerapi.ControlMessage_ConfigCommand{
			ConfigCommand: configCommand,
		},
	}

	zlog.Info().
		Int("connections_count", len(apiConnections)).
		Int("subscriptions_count", len(apiSubscriptions)).
		Str("message_id", messageID).
		Msg("Sending configuration command to registered node")

	// Send configuration command to the node
	err = s.nodeCommandHandler.SendMessage(nodeID, msg)
	if err != nil {
		return fmt.Errorf("failed to send configuration command to node %s: %w", nodeID, err)
	}

	// Wait for ACK response from the node
	response, err := s.nodeCommandHandler.WaitForResponse(
		nodeID,
		reflect.TypeOf(&controllerapi.ControlMessage_Ack{}),
	)
	if err != nil {
		return fmt.Errorf("failed to receive ACK response from node %s: %w", nodeID, err)
	}

	// Validate ACK response
	if ack := response.GetAck(); ack != nil {
		if !ack.Success {
			zlog.Error().
				Strs("error_messages", ack.Messages).
				Msg("Node registration configuration failed")
			return fmt.Errorf("node registration configuration failed: %v", ack.Messages)
		}

		zlog.Info().
			Str("original_message_id", ack.OriginalMessageId).
			Strs("ack_messages", ack.Messages).
			Msg("Node registration configuration completed successfully")
	} else {
		return fmt.Errorf("received invalid ACK response from node %s", nodeID)
	}

	zlog.Info().Msg("Node registration synchronization completed successfully")
	return nil
}
