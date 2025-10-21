package routes

import (
	"context"
	"fmt"
	"reflect"

	"github.com/google/uuid"
	"github.com/rs/zerolog"
	"k8s.io/client-go/util/workqueue"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
)

type RouteReconcileRequest struct {
	NodeID string
}

// RouteReconciler handles node registration events by synchronizing
// stored connections and subscriptions from the database to newly registered nodes
type RouteReconciler struct {
	dbService          db.DataAccess
	nodeCommandHandler nodecontrol.NodeCommandHandler
	queue              workqueue.TypedRateLimitingInterface[RouteReconcileRequest]
	threadName         string
	maxRequeues        int
}

// NewRouteReconciler creates a new instance of RouteReconciler
func NewRouteReconciler(
	threadName string,
	maxRequeues int,
	queue workqueue.TypedRateLimitingInterface[RouteReconcileRequest],
	dbService db.DataAccess,
	nodeCommandHandler nodecontrol.NodeCommandHandler,
) *RouteReconciler {
	return &RouteReconciler{
		dbService:          dbService,
		nodeCommandHandler: nodeCommandHandler,
		queue:              queue,
		threadName:         threadName,
		maxRequeues:        maxRequeues,
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
				if s.queue.NumRequeues(req) < s.maxRequeues {
					s.queue.AddRateLimited(req)
				} else {
					zlog.Warn().Msgf("Max retries reached for request: %v, dropping from queue", req)
					s.queue.Forget(req)
				}
			}
		}()
	}
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

	routes := s.dbService.GetRoutesForNodeID(nodeID)
	for _, route := range routes {
		// create connection and subscription for each route
		apiSubscription := &controllerapi.Subscription{
			ConnectionId: route.DestEndpoint, // Use endpoint as connection ID
			Component_0:  route.Component0,
			Component_1:  route.Component1,
			Component_2:  route.Component2,
			Id:           route.ComponentID,
		}
		if route.DestNodeID != "" {
			apiSubscription.NodeId = &route.DestNodeID
		}

		if route.Deleted {
			apiSubscriptionsToDelete = append(apiSubscriptionsToDelete, apiSubscription)
			continue
		}
		apiConnections[route.DestEndpoint] = &controllerapi.Connection{
			ConnectionId: route.DestEndpoint,
			ConfigData:   route.ConnConfigData,
		}
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

	// Wait for ConfigCommandAck response from the node
	response, err := s.nodeCommandHandler.WaitForResponse(ctx,
		nodeID,
		reflect.TypeOf(&controllerapi.ControlMessage_ConfigCommandAck{}),
		messageID,
	)
	if err != nil {
		return fmt.Errorf("failed to receive ConfigCommandAck response from node %s: %w", nodeID, err)
	}

	// Handle ConfigCommandAck response
	if ack := response.GetConfigCommandAck(); ack != nil {
		// Create a map of connection errors for quick lookup
		connectionErrors := make(map[string]string)
		for _, connAck := range ack.GetConnectionsStatus() {
			if !connAck.Success {
				connectionErrors[connAck.ConnectionId] = connAck.ErrorMsg
			}
		}

		for _, subAck := range ack.GetSubscriptionsStatus() {
			// get route key to find the corresponding route
			routeKey := s.getSubscriptionToRouteKey(nodeID, subAck.Subscription)

			// Fetch route from database using the route key
			route := s.dbService.GetRouteByID(routeKey)
			if route == nil {
				zlog.Warn().
					Str("route_key", routeKey).
					Msg("Route not found for subscription acknowledgment")
				continue
			}

			if subAck.Success {
				// Success case: mark route as applied or delete if it was marked as deleted
				if route.Deleted {
					// Route was for deletion, so delete it from database
					if err := s.dbService.DeleteRoute(routeKey); err != nil {
						zlog.Error().
							Err(err).
							Str("route_key", routeKey).
							Msg("Failed to delete route from database")
						return fmt.Errorf("failed to delete route %s: %w", routeKey, err)
					}
					zlog.Info().
						Str("route_key", routeKey).
						Msg("Successfully deleted route")
				} else {
					// Route was for creation/update, mark as applied
					if err := s.dbService.MarkRouteAsApplied(routeKey); err != nil {
						zlog.Error().
							Err(err).
							Str("route_key", routeKey).
							Msg("Failed to mark route as applied")
						return fmt.Errorf("failed to mark route %s as applied: %w", routeKey, err)
					}
					zlog.Debug().
						Str("route_key", routeKey).
						Msg("Successfully marked route as applied")
				}
			} else {
				// Failure case: mark route as failed with error message
				failedMsg := subAck.ErrorMsg

				// Check if there's a connection error with the same connectionID
				if connErr, exists := connectionErrors[subAck.Subscription.ConnectionId]; exists {
					failedMsg = connErr
				}

				if err := s.dbService.MarkRouteAsFailed(routeKey, failedMsg); err != nil {
					zlog.Error().
						Err(err).
						Str("route_key", routeKey).
						Str("error_msg", failedMsg).
						Msg("Failed to mark route as failed")
					return fmt.Errorf("failed to mark route %s as failed: %w", routeKey, err)
				}
				zlog.Info().
					Str("route_key", routeKey).
					Str("error_msg", failedMsg).
					Msg("Marked route as failed")
			}
		}

		zlog.Info().
			Str("original_message_id", ack.OriginalMessageId).
			Str("node_id", nodeID).
			Msg("Configuration command processing completed")

	} else {
		return fmt.Errorf("received invalid ConfigCommandAck response from node %s", nodeID)
	}

	return nil
}

// getSubscriptionErrorKey creates a unique key for subscription error mapping
func (s *RouteReconciler) getSubscriptionToRouteKey(sourceNodeID string, sub *controllerapi.Subscription) string {
	nodeID := ""
	if sub.NodeId != nil {
		nodeID = *sub.NodeId
	}
	return fmt.Sprintf("%s:%s/%s/%s/%v->%s[%s]", sourceNodeID,
		sub.Component_0, sub.Component_1, sub.Component_2, sub.Id, nodeID, sub.ConnectionId)
}
