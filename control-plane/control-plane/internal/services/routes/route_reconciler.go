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
	var deletedRoutes []string

	// Track route ID to subscription mapping for error handling
	subscriptionToRouteID := make(map[string]string)
	deleteSubscriptionToRouteID := make(map[string]string)

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

		// Create subscription key for mapping
		subscriptionKey := s.getSubscriptionKey(apiSubscription)

		if route.Deleted {
			apiSubscriptionsToDelete = append(apiSubscriptionsToDelete, apiSubscription)
			deletedRoutes = append(deletedRoutes, route.GetID())
			deleteSubscriptionToRouteID[subscriptionKey] = route.GetID()
			continue
		}
		apiConnections[route.DestEndpoint] = &controllerapi.Connection{
			ConnectionId: route.DestEndpoint,
			ConfigData:   route.ConnConfigData,
		}
		apiSubscriptions = append(apiSubscriptions, apiSubscription)
		subscriptionToRouteID[subscriptionKey] = route.GetID()
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
		// Handle subscription add failures
		if err := s.handleSubscriptionAddFailures(ctx, ack, subscriptionToRouteID, zlog); err != nil {
			return fmt.Errorf("failed to handle subscription add failures: %w", err)
		}

		// Handle subscription delete failures
		if err := s.handleSubscriptionDeleteFailures(ctx, ack, deleteSubscriptionToRouteID, zlog); err != nil {
			return fmt.Errorf("failed to handle subscription delete failures: %w", err)
		}

		// Mark successful routes as applied and delete successful deleted routes
		if err := s.handleSuccessfulRoutes(ctx, ack, subscriptionToRouteID, deleteSubscriptionToRouteID,
			deletedRoutes, zlog); err != nil {
			return fmt.Errorf("failed to handle successful routes: %w", err)
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

// getSubscriptionKey creates a unique key for subscription mapping
func (s *RouteReconciler) getSubscriptionKey(sub *controllerapi.Subscription) string {
	idStr := ""
	if sub.Id != nil {
		idStr = fmt.Sprintf(":%d", sub.Id.GetValue())
	}
	return fmt.Sprintf("%s:%s:%s:%s%s", sub.ConnectionId, sub.Component_0, sub.Component_1, sub.Component_2, idStr)
}

// handleSubscriptionAddFailures processes failed subscription additions
func (s *RouteReconciler) handleSubscriptionAddFailures(_ context.Context,
	ack *controllerapi.ConfigurationCommandAck, subscriptionToRouteID map[string]string, zlog zerolog.Logger) error {
	// Create a map of connection errors for quick lookup
	connectionErrors := make(map[string]string)
	for _, connErr := range ack.GetConnectionsFailedToCreate() {
		connectionErrors[connErr.GetConnectionId()] = connErr.GetErrorMsg()
	}

	// Handle failed subscription additions
	for _, subErr := range ack.GetSubscriptionsFailedToSet() {
		subscriptionKey := s.getSubscriptionErrorKey(subErr)
		routeID, exists := subscriptionToRouteID[subscriptionKey]
		if !exists {
			zlog.Warn().Msgf("No route mapping found for failed subscription: %s", subscriptionKey)
			continue
		}

		// Determine the failure message - prefer connection error if available
		failedMsg := subErr.GetErrorMsg()
		connID := subErr.GetSubscription().GetConnectionId()
		if connErr, hasConnErr := connectionErrors[connID]; hasConnErr {
			failedMsg = connErr
		}

		// Mark the corresponding route as failed
		if err := s.dbService.MarkRouteAsFailed(routeID, failedMsg); err != nil {
			zlog.Error().Err(err).Msgf("Failed to mark route %s as failed", routeID)
			return fmt.Errorf("failed to mark route %s as failed: %w", routeID, err)
		}

		zlog.Info().Str("route_id", routeID).Str("error", failedMsg).Msg(
			"Marked route as failed due to subscription add failure")
	}

	return nil
}

// handleSubscriptionDeleteFailures processes failed subscription deletions
func (s *RouteReconciler) handleSubscriptionDeleteFailures(_ context.Context,
	ack *controllerapi.ConfigurationCommandAck, deleteSubscriptionToRouteID map[string]string, zlog zerolog.Logger) error {
	// Handle failed subscription deletions
	for _, subErr := range ack.GetSubscriptionsFailedToDelete() {
		subscriptionKey := s.getSubscriptionErrorKey(subErr)
		routeID, exists := deleteSubscriptionToRouteID[subscriptionKey]
		if !exists {
			zlog.Warn().Msgf("No route mapping found for failed deletion subscription: %s", subscriptionKey)
			continue
		}

		failedMsg := subErr.GetErrorMsg()

		// Mark the route as failed
		if err := s.dbService.MarkRouteAsFailed(routeID, failedMsg); err != nil {
			zlog.Error().Err(err).Msgf("Failed to mark route %s as failed during deletion", routeID)
			return fmt.Errorf("failed to mark route %s as failed during deletion: %w", routeID, err)
		}

		zlog.Info().Str("route_id", routeID).Str("error", failedMsg).Msg(
			"Marked route as failed due to subscription delete failure")
	}

	return nil
}

// handleSuccessfulRoutes marks successful routes as applied and deletes successful deleted routes
func (s *RouteReconciler) handleSuccessfulRoutes(_ context.Context,
	ack *controllerapi.ConfigurationCommandAck, subscriptionToRouteID, deleteSubscriptionToRouteID map[string]string,
	_ []string, zlog zerolog.Logger) error {

	// Create sets of failed subscription keys for quick lookup
	failedAddSubscriptionKeys := make(map[string]bool)
	failedDeleteSubscriptionKeys := make(map[string]bool)

	for _, subErr := range ack.GetSubscriptionsFailedToSet() {
		failedAddSubscriptionKeys[s.getSubscriptionErrorKey(subErr)] = true
	}

	for _, subErr := range ack.GetSubscriptionsFailedToDelete() {
		failedDeleteSubscriptionKeys[s.getSubscriptionErrorKey(subErr)] = true
	}

	// Mark successful add routes as applied
	for subscriptionKey, routeID := range subscriptionToRouteID {
		if !failedAddSubscriptionKeys[subscriptionKey] {
			if err := s.dbService.MarkRouteAsApplied(routeID); err != nil {
				zlog.Error().Err(err).Msgf("Failed to mark route %s as applied", routeID)
				return fmt.Errorf("failed to mark route %s as applied: %w", routeID, err)
			}
			zlog.Debug().Str("route_id", routeID).Msg("Marked route as applied")
		}
	}

	// Delete successful deleted routes
	for subscriptionKey, routeID := range deleteSubscriptionToRouteID {
		if !failedDeleteSubscriptionKeys[subscriptionKey] {
			if err := s.dbService.DeleteRoute(routeID); err != nil {
				zlog.Error().Err(err).Msgf("Failed to delete route %s from database", routeID)
				return fmt.Errorf("failed to delete route %s: %w", routeID, err)
			}
			zlog.Debug().Str("route_id", routeID).Msg("Successfully deleted route")
		}
	}

	return nil
}

// getSubscriptionErrorKey creates a unique key for subscription error mapping
func (s *RouteReconciler) getSubscriptionErrorKey(subErr *controllerapi.SubscriptionError) string {
	idStr := ""
	if subErr.Subscription.Id != nil {
		idStr = fmt.Sprintf(":%d", subErr.Subscription.Id.GetValue())
	}
	connID := subErr.GetSubscription().GetConnectionId()
	return fmt.Sprintf("%s:%s:%s:%s%s", connID,
		subErr.Subscription.Component_0, subErr.Subscription.Component_1, subErr.Subscription.Component_2, idStr)
}
