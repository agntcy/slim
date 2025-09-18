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
	"k8s.io/client-go/util/workqueue"
)

type RouteReconcileRequest struct {
	NodeID string
}

// RouteReconciler handles node registration events by synchronizing
// stored connections and subscriptions from the database to newly registered nodes
type RouteReconciler struct {
	dbService          db.DataAccess
	nodeCommandHandler nodecontrol.NodeCommandHandler
	queue              *workqueue.Typed[RouteReconcileRequest]
	threadName         string
}

// NewRouteReconciler creates a new instance of RouteReconciler
func NewRouteReconciler(
	threadName string,
	queue *workqueue.Typed[RouteReconcileRequest],
	dbService db.DataAccess,
	nodeCommandHandler nodecontrol.NodeCommandHandler,
) *RouteReconciler {
	return &RouteReconciler{
		dbService:          dbService,
		nodeCommandHandler: nodeCommandHandler,
		queue:              queue,
		threadName:         threadName,
	}
}

func (s *RouteReconciler) Run(ctx context.Context) {
	zlog := zerolog.Ctx(ctx).With().Str("thread_name", s.threadName).Logger()
	zlog.Info().Msg("Starting Route Reconciler")
	for {
		req, shutdown := s.queue.Get()
		if shutdown {
			zlog.Info().Msg("Route Reconciler queue is shutting down")
			return
		}
		func() {
			defer s.queue.Done(req)
			if err := s.handleRequest(ctx, req); err != nil {
				zlog.Error().Err(err).Msg("Failed to process route reconciliation request")
				// Optionally requeue the request for retry
				s.queue.Add(req)
			}
		}()
	}
}

func (s *RouteReconciler) getConnectionDetails(route db.Route) (controllerapi.Connection, error) {
	if route.DestNodeID == "" {
		return controllerapi.Connection{
			ConnectionId: route.DestEndpoint,
			ConfigData:   route.ConnConfigData,
		}, nil
	}

	destNode, err := s.dbService.GetNode(route.DestNodeID)
	if err != nil {
		return controllerapi.Connection{}, fmt.Errorf("failed to fetch destination node %s: %w", route.DestNodeID, err)
	}
	// TODO handle multiple connections, external and internal etc.
	if len(destNode.ConnDetails) == 0 {
		return controllerapi.Connection{}, fmt.Errorf("no connections found for destination node %s", destNode.ID)
	}
	connDetail := destNode.ConnDetails[0]

	configData, err := generateConfigData(connDetail)
	if err != nil {
		return controllerapi.Connection{}, fmt.Errorf("failed to generate config data for route %v: %w", route, err)
	}

	return controllerapi.Connection{
		ConnectionId: connDetail.Endpoint, // Use endpoint as connection ID
		ConfigData:   configData,
	}, nil
}

// handleRequest processes a node registration request
// When a node is registered, it fetches all connections and subscriptions and routes from DbService
// and wraps them in a ConfigurationCommand message to send to the node
func (s *RouteReconciler) handleRequest(ctx context.Context, req RouteReconcileRequest) error {
	nodeID := req.NodeID
	zlog := zerolog.Ctx(ctx).With().Str("node_id", nodeID).Logger()

	// reconcile only connected nodes
	if nodeStatus, err := s.nodeCommandHandler.GetConnectionStatus(ctx, nodeID); err != nil {
		return fmt.Errorf("failed to get connection status for node %s: %w", nodeID, err)
	} else if nodeStatus != nodecontrol.NodeStatusConnected {
		zlog.Info().Msgf("Node %s is not connected, skipping reconciliation", nodeID)
		return nil
	}

	zlog.Info().Msgf("Sending routes to registered node %s", nodeID)

	apiConnections := make(map[string]*controllerapi.Connection, 0)
	var apiSubscriptions []*controllerapi.Subscription
	var apiSubscriptionsToDelete []*controllerapi.Subscription
	var deletedRoutes []string

	routes := s.dbService.GetRoutesForNodeID(nodeID)
	for _, route := range routes {
		// create connection and subscription for each route
		apiConnection, err := s.getConnectionDetails(route)
		if err != nil {
			zlog.Error().Err(err).Msgf("Failed to get connection details for route %v, skipping", route)
			continue
		}
		apiSubscription := &controllerapi.Subscription{
			ConnectionId: apiConnection.ConnectionId, // Use endpoint as connection ID
			Component_0:  route.Component0,
			Component_1:  route.Component1,
			Component_2:  route.Component2,
			Id:           route.ComponentID,
		}
		if route.Deleted {
			apiSubscriptionsToDelete = append(apiSubscriptionsToDelete, apiSubscription)
			deletedRoutes = append(deletedRoutes, route.GetID())
			continue
		}
		apiConnections[apiConnection.ConnectionId] = &apiConnection
		apiSubscriptions = append(apiSubscriptions, apiSubscription)
	}

	// convert map to slice
	apiConnectionsSlice := make([]*controllerapi.Connection, 0, len(apiConnections))
	for _, conn := range apiConnections {
		apiConnectionsSlice = append(apiConnectionsSlice, conn)
	}

	// Create configuration command with all stored connections and subscriptions
	configCommand := &controllerapi.ConfigurationCommand{
		ConnectionsToCreate:   apiConnectionsSlice,
		SubscriptionsToSet:    apiSubscriptions,
		SubscriptionsToDelete: apiSubscriptionsToDelete,
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
		Int("subscriptions_to_delete_count", len(apiSubscriptionsToDelete)).
		Str("message_id", messageID).
		Msg("Sending configuration command to registered node")

	// Send configuration command to the node
	err := s.nodeCommandHandler.SendMessage(ctx, nodeID, msg)
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
				Msgf("Sending route configs for node %s failed", nodeID)
			return fmt.Errorf("sending route config for node %s failed: %v", nodeID, ack.Messages)
		}

		// If there are any deleted routes, remove them from the database
		if len(deletedRoutes) > 0 {
			for _, routeID := range deletedRoutes {
				if err := s.dbService.DeleteRoute(routeID); err != nil {
					zlog.Error().Msgf("failed to delete route %s from database: %v", routeID, err)
				}
			}
		}

		zlog.Info().
			Str("original_message_id", ack.OriginalMessageId).
			Strs("ack_messages", ack.Messages).
			Str("node_id", nodeID).
			Msg("Sending routes completed successfully")

	} else {
		return fmt.Errorf("received invalid ACK response from node %s", nodeID)
	}

	return nil
}

func generateConfigData(detail db.ConnectionDetails) (string, error) {
	truev := true
	falsev := false
	config := ConnectionConfig{
		Endpoint: detail.Endpoint,
	}
	if !detail.MTLSRequired {
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
		HTTPClient2Keepalive: stringPtr("2h"),
		KeepAliveWhileIdle:   &falsev,
		TCPKeepalive:         stringPtr("20s"),
		Timeout:              stringPtr("20s"),
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
