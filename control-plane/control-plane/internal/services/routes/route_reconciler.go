package routes

import (
	"context"
	"fmt"
	"reflect"
	"strings"
	"time"

	"github.com/google/uuid"
	"github.com/rs/zerolog"
	"k8s.io/client-go/util/workqueue"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
)

type RouteReconcileRequest struct {
	NodeID string
}

// RouteReconciler handles node registration events by synchronizing
// stored connections and subscriptions from the database to newly registered nodes
type RouteReconciler struct {
	reconcileConfig    config.ReconcilerConfig
	threadName         string
	dbService          db.DataAccess
	nodeCommandHandler nodecontrol.NodeCommandHandler
	runningReconciles  int
	queue              workqueue.TypedRateLimitingInterface[RouteReconcileRequest]
}

// NewRouteReconciler creates a new instance of RouteReconciler
func NewRouteReconciler(
	threadName string,
	reconcileConfig config.ReconcilerConfig,
	queue workqueue.TypedRateLimitingInterface[RouteReconcileRequest],
	dbService db.DataAccess,
	nodeCommandHandler nodecontrol.NodeCommandHandler,
) *RouteReconciler {
	return &RouteReconciler{
		reconcileConfig:    reconcileConfig,
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
		zlog.Debug().Msgf("GOT REQ: %s", req)
		if shutdown {
			zlog.Info().Msg("Route Reconciler queue is shutting down")
			return
		}

		// Check if we've reached the max parallel reconciles limit
		if s.runningReconciles >= s.reconcileConfig.MaxNumOfParallelReconciles {
			zlog.Debug().
				Int("running_reconciles", s.runningReconciles).
				Int("max_parallel", s.reconcileConfig.MaxNumOfParallelReconciles).
				Str("node_id", req.NodeID).
				Msg("Max parallel reconciles reached, re-queuing with delay")
			s.queue.Done(req)
			s.queue.AddAfter(req, 5*time.Second)
			continue
		}

		s.runningReconciles++
		zlog.Debug().
			Str("node_id", req.NodeID).
			Int("running_reconciles", s.runningReconciles).
			Msg("Starting reconcile in goroutine")

		// Start runReconcile in a separate goroutine
		go s.runReconcile(ctx, req)
	}
}

func (s *RouteReconciler) runReconcile(ctx context.Context, req RouteReconcileRequest) {
	zlog := zerolog.Ctx(ctx).With().Str("thread_name", s.threadName).Logger()
	defer func() {
		s.queue.Done(req)
		s.runningReconciles--
		zlog.Debug().
			Str("node_id", req.NodeID).
			Msg("Reconcile completed")
	}()

	if err := s.handleRequest(ctx, req); err != nil {
		zlog.Error().Err(err).Msg("Failed to process route reconciliation request")
		// Optionally requeue the request for retry
		if s.queue.NumRequeues(req) < s.reconcileConfig.MaxRequeues {
			zlog.Debug().
				Str("node_id", req.NodeID).
				Int("requeue_count", s.queue.NumRequeues(req)).
				Msg("Re-queuing failed request with rate limit")
			s.queue.AddRateLimited(req)
		} else {
			zlog.Warn().
				Str("node_id", req.NodeID).
				Int("max_requeues", s.reconcileConfig.MaxRequeues).
				Msgf("Max retries reached for request: %v, dropping from queue", req)
			s.queue.Forget(req)
		}
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

	var apiSubscriptions []*controllerapi.Subscription
	var apiSubscriptionsToDelete []*controllerapi.Subscription
	needsRequeue := false

	routes := s.dbService.GetRoutesForNodeID(nodeID)
	for _, route := range routes {
		linkID := route.LinkID
		// create connection and subscription for each route
		apiSubscription := &controllerapi.Subscription{
			ConnectionId: linkID,
			Component_0:  route.Component0,
			Component_1:  route.Component1,
			Component_2:  route.Component2,
			Id:           route.ComponentID,
			LinkId:       &linkID,
		}
		if route.DestNodeID != "" {
			apiSubscription.NodeId = &route.DestNodeID
		}

		if route.Deleted {
			apiSubscriptionsToDelete = append(apiSubscriptionsToDelete, apiSubscription)
			continue
		}
		link, lerr := s.dbService.GetLink(route.LinkID, route.SourceNodeID, route.DestNodeID)
		if lerr != nil || link == nil {
			zlog.Warn().
				Str("route", route.String()).
				Msgf("Skipping route due to missing link: %v", lerr)
			continue
		}
		if link.Status == db.LinkStatusFailed {
			errMsg := link.StatusMsg
			if errMsg == "" {
				errMsg = "link configuration failed"
			}
			if err := s.dbService.MarkRouteAsFailed(route.ID, errMsg); err != nil {
				return fmt.Errorf("failed to mark route %s as failed due to link status: %w", route.String(), err)
			}
			continue
		}
		if link.Status != db.LinkStatusApplied {
			zlog.Debug().Str("link_id", linkID).Msg("Skipping route until link is applied")
			needsRequeue = true
			continue
		}
		apiSubscriptions = append(apiSubscriptions, apiSubscription)
	}

	// Create configuration command with all stored connections and subscriptions
	if len(apiSubscriptions) == 0 && len(apiSubscriptionsToDelete) == 0 {
		zlog.Debug().Msg("No subscription updates to send for node")
		if needsRequeue {
			zlog.Debug().Msg("Re-queueing route reconciliation while waiting for link(s) to apply")
			s.queue.AddAfter(req, 1*time.Second)
		}
		return nil
	}

	// Create configuration command with all stored connections and subscriptions
	configCommand := &controllerapi.ConfigurationCommand{
		ConnectionsToCreate:   []*controllerapi.Connection{},
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
		Int("connections_count", 0).
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
		for _, subAck := range ack.GetSubscriptionsStatus() {
			// find corresponding route in the database
			destNodeID := ""
			if subAck.Subscription.NodeId != nil {
				destNodeID = *subAck.Subscription.NodeId
			}
			route, rerr := s.dbService.GetRouteForSrcAndDestinationAndName(nodeID, subAck.Subscription.Component_0,
				subAck.Subscription.Component_1, subAck.Subscription.Component_2, subAck.Subscription.Id,
				destNodeID, subAck.Subscription.GetLinkId())
			if rerr != nil {
				zlog.Warn().
					Str("subscription", subAck.Subscription.String()).
					Msgf("Route not found for subscription acknowledgment: %s", rerr.Error())
				continue
			}

			if subAck.Success {
				// Success case: mark route as applied or delete if it was marked as deleted
				if route.Deleted {
					// Route was for deletion, so delete it from database
					if err := s.dbService.DeleteRoute(route.ID); err != nil {
						zlog.Error().
							Err(err).
							Str("route_key", route.String()).
							Msg("Failed to delete route from database")
						return fmt.Errorf("failed to delete route %s: %w", route.String(), err)
					}
					zlog.Info().
						Str("route_key", route.String()).
						Msg("Successfully deleted route")
				} else {
					// Route was for creation/update, mark as applied
					if err := s.dbService.MarkRouteAsApplied(route.ID); err != nil {
						zlog.Error().
							Err(err).
							Str("route_key", route.String()).
							Msg("Failed to mark route as applied")
						return fmt.Errorf("failed to mark route %s as applied: %w", route.String(), err)
					}
					zlog.Debug().
						Str("route_key", route.String()).
						Msg("Successfully marked route as applied")
				}
			} else {
				// Failure case: mark route as failed with error message
				failedMsg := subAck.ErrorMsg

				if err := s.dbService.MarkRouteAsFailed(route.ID, failedMsg); err != nil {
					zlog.Error().
						Err(err).
						Str("route_key", route.String()).
						Str("error_msg", failedMsg).
						Msg("Failed to mark route as failed")
					return fmt.Errorf("failed to mark route %s as failed: %w", route, err)
				}
				zlog.Error().
					Str("route_key", route.String()).
					Str("error_msg", failedMsg).
					Msg("Marked route as failed")
				if shouldRetryMissingLinkConnection(failedMsg) {
					needsRequeue = true
					zlog.Warn().
						Str("route_key", route.String()).
						Str("error_msg", failedMsg).
						Msg("Re-queueing route reconciliation due to missing connection/link_id error")
				}
			}
		}

		zlog.Info().
			Str("original_message_id", ack.OriginalMessageId).
			Str("node_id", nodeID).
			Msg("Configuration command processing completed")

	} else {
		return fmt.Errorf("received invalid ConfigCommandAck response from node %s", nodeID)
	}

	if needsRequeue {
		zlog.Debug().Msg("Re-queueing route reconciliation while waiting for link(s) to apply")
		s.queue.AddAfter(req, 1*time.Second)
	}

	return nil
}

func shouldRetryMissingLinkConnection(errMsg string) bool {
	lower := strings.ToLower(errMsg)
	return strings.Contains(lower, "connection with link_id") && strings.Contains(lower, "not found")
}
