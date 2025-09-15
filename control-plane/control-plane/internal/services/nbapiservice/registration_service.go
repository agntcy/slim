package nbapiservice

import (
	"bytes"
	"context"
	"encoding/json"
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
			Component_0:  sub.Component0,
			Component_1:  sub.Component1,
			Component_2:  sub.Component2,
			Id:           sub.ComponentID,
		}
		apiSubscriptions = append(apiSubscriptions, apiSubscription)
	}

	zlog.Debug().Msgf("generating routes for node %s", nodeID)
	routes, err := s.dbService.GetRoutesForNodeID(nodeID)
	if err != nil {
		return fmt.Errorf("failed to fetch routes for node %s: %w", nodeID, err)
	}
	for _, route := range routes {
		destNode, err := s.dbService.GetNode(route.DestNodeID)
		if err != nil {
			return fmt.Errorf("failed to fetch destination node %s: %w", destNode, err)
		}
		//TODO handle multiple connections
		if len(destNode.ConnDetails) == 0 {
			zlog.Error().Msgf("no connections found for destination node %s", destNode.ID)
			continue
		}
		connDetail := destNode.ConnDetails[0]

		configData, err := generateConfigData(route, connDetail)
		if err != nil {
			return fmt.Errorf("failed to generate config data for route %v: %w", route, err)
		}

		// create connection and subscription for each route
		apiConnection := &controllerapi.Connection{
			ConnectionId: connDetail.Endpoint, // Use endpoint as connection ID
			ConfigData:   configData,
		}
		apiConnections = append(apiConnections, apiConnection)
		apiSubscription := &controllerapi.Subscription{
			ConnectionId: connDetail.Endpoint, // Use endpoint as connection ID
			Component_0:  route.Component0,
			Component_1:  route.Component1,
			Component_2:  route.Component2,
			Id:           route.ComponentID,
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
	err = s.nodeCommandHandler.SendMessage(ctx, nodeID, msg)
	if err != nil {
		return fmt.Errorf("failed to send configuration command to node %s: %w", nodeID, err)
	}

	// Wait for ACK response from the node
	response, err := s.nodeCommandHandler.WaitForResponse(ctx,
		nodeID,
		reflect.TypeOf(&controllerapi.ControlMessage_Ack{}),
		messageID,
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

func generateConfigData(route db.Route, detail db.ConnectionDetails) (string, error) {
	truev := true
	falsev := false
	config := ConnectionConfig{
		Endpoint: detail.Endpoint,
	}
	if detail.MtlsRequired {
		config.TLS = &TLS{Insecure: &truev}
	} else {
		config.TLS = &TLS{
			Insecure: &falsev,
			CERTFile: stringPtr("/svids/tls.crt"),
			KeyFile:  stringPtr("/svids/tls.key"),
			CAFile:   stringPtr("/svids/svid_bundle.pem"),
		}
	}
	var bufferSize int64 = 1024
	config.BufferSize = &bufferSize
	gzip := Gzip
	config.Compression = &gzip
	config.ConnectTimeout = stringPtr("10s")
	config.Headers = map[string]string{
		"x-custom-header": "value",
	}

	config.Keepalive = &KeepaliveClass{
		Http2Keepalive:     stringPtr("2h"),
		KeepAliveWhileIdle: &falsev,
		TCPKeepalive:       stringPtr("20s"),
		Timeout:            stringPtr("20s"),
	}
	config.Origin = stringPtr("https://client.example.com")
	config.RateLimit = stringPtr("20/60")
	config.RequestTimeout = stringPtr("30s")

	// render struct as json
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	err := enc.Encode(config)
	if err != nil {
		return "", fmt.Errorf("failed to encode connection config: %w", err)
	}
	fmt.Println("Generated connection config:")
	fmt.Println(buf.String())

	return buf.String(), nil
}

func stringPtr(s string) *string {
	return &s
}
