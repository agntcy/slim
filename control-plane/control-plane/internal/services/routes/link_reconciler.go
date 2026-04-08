package routes

import (
	"context"
	"encoding/json"
	"fmt"
	"reflect"
	"time"

	"github.com/google/uuid"
	"github.com/rs/zerolog"
	"k8s.io/client-go/util/workqueue"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
)

type LinkReconcileRequest struct {
	NodeID string
}

type LinkReconciler struct {
	reconcileConfig    config.ReconcilerConfig
	threadName         string
	dbService          db.DataAccess
	nodeCommandHandler nodecontrol.NodeCommandHandler
	runningReconciles  int
	queue              workqueue.TypedRateLimitingInterface[LinkReconcileRequest]
	routeQueue         workqueue.TypedRateLimitingInterface[RouteReconcileRequest]
}

func NewLinkReconciler(
	threadName string,
	reconcileConfig config.ReconcilerConfig,
	queue workqueue.TypedRateLimitingInterface[LinkReconcileRequest],
	routeQueue workqueue.TypedRateLimitingInterface[RouteReconcileRequest],
	dbService db.DataAccess,
	nodeCommandHandler nodecontrol.NodeCommandHandler,
) *LinkReconciler {
	return &LinkReconciler{
		reconcileConfig:    reconcileConfig,
		threadName:         threadName,
		dbService:          dbService,
		nodeCommandHandler: nodeCommandHandler,
		queue:              queue,
		routeQueue:         routeQueue,
	}
}

func (s *LinkReconciler) Run(ctx context.Context) {
	zlog := zerolog.Ctx(ctx).With().Str("thread_name", s.threadName).Logger()
	zlog.Info().Msg("Starting Link Reconciler")

	for {
		req, shutdown := s.queue.Get()
		if shutdown {
			zlog.Info().Msg("Link Reconciler queue is shutting down")
			return
		}
		if s.runningReconciles >= s.reconcileConfig.MaxNumOfParallelReconciles {
			s.queue.Done(req)
			s.queue.AddAfter(req, 5*time.Second)
			continue
		}
		s.runningReconciles++
		go s.runReconcile(ctx, req)
	}
}

func (s *LinkReconciler) runReconcile(ctx context.Context, req LinkReconcileRequest) {
	zlog := zerolog.Ctx(ctx).With().Str("thread_name", s.threadName).Logger()
	defer func() {
		s.queue.Done(req)
		s.runningReconciles--
	}()

	if err := s.handleRequest(ctx, req); err != nil {
		zlog.Error().Err(err).Msg("Failed to process link reconciliation request")
		if s.queue.NumRequeues(req) < s.reconcileConfig.MaxRequeues {
			s.queue.AddRateLimited(req)
		} else {
			s.queue.Forget(req)
		}
	}
}

func (s *LinkReconciler) handleRequest(ctx context.Context, req LinkReconcileRequest) error {
	nodeID := req.NodeID
	zlog := zerolog.Ctx(ctx).With().Str("node_id", nodeID).Logger()
	if nodeStatus, err := s.nodeCommandHandler.GetConnectionStatus(ctx, nodeID); err != nil {
		return fmt.Errorf("failed to get connection status for node %s: %w", nodeID, err)
	} else if nodeStatus != nodecontrol.NodeStatusConnected {
		return nil
	}

	links := s.dbService.GetLinksForNode(nodeID)
	connMap := make(map[string]*controllerapi.Connection)
	impactedNodes := map[string]struct{}{}
	for _, link := range links {
		if link.SourceNodeID != nodeID {
			continue
		}
		if link.Deleted {
			routes := s.dbService.GetRoutesByLinkID(link.LinkID)
			for _, route := range routes {
				if err := s.dbService.DeleteRoute(route.ID); err != nil {
					return fmt.Errorf("failed to delete dependent route for deleted link %s: %w", link.LinkID, err)
				}
			}
			if err := s.dbService.DeleteLink(link); err != nil {
				return fmt.Errorf("failed to delete link %s after delete-ack flow: %w", link.LinkID, err)
			}
			continue
		}
		configData := link.ConnConfigData
		var cfg map[string]interface{}
		if json.Unmarshal([]byte(configData), &cfg) == nil {
			if _, ok := cfg["link_id"]; !ok {
				cfg["link_id"] = link.LinkID
				if updated, err := json.Marshal(cfg); err == nil {
					configData = string(updated)
				}
			}
		}
		if _, exists := connMap[link.LinkID]; !exists {
			connMap[link.LinkID] = &controllerapi.Connection{
				ConnectionId: link.LinkID,
				ConfigData:   configData,
			}
		}
	}

	connections := make([]*controllerapi.Connection, 0, len(connMap))
	for _, c := range connMap {
		connections = append(connections, c)
	}
	if len(connections) == 0 {
		return nil
	}

	messageID := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageID,
		Payload: &controllerapi.ControlMessage_ConfigCommand{
			ConfigCommand: &controllerapi.ConfigurationCommand{
				ConnectionsToCreate:   connections,
				SubscriptionsToSet:    []*controllerapi.Subscription{},
				SubscriptionsToDelete: []*controllerapi.Subscription{},
			},
		},
	}
	if err := s.nodeCommandHandler.SendMessage(ctx, nodeID, msg); err != nil {
		return fmt.Errorf("failed to send link configuration command to node %s: %w", nodeID, err)
	}

	response, err := s.nodeCommandHandler.WaitForResponse(
		ctx,
		nodeID,
		reflect.TypeOf(&controllerapi.ControlMessage_ConfigCommandAck{}),
		messageID,
	)
	if err != nil {
		return fmt.Errorf("failed to receive ConfigCommandAck response from node %s: %w", nodeID, err)
	}

	ack := response.GetConfigCommandAck()
	if ack == nil {
		return fmt.Errorf("received invalid ConfigCommandAck response from node %s", nodeID)
	}

	linkStatusByID := make(map[string]struct {
		status db.LinkStatus
		msg    string
	})
	for _, connAck := range ack.GetConnectionsStatus() {
		if connAck.Success {
			linkStatusByID[connAck.ConnectionId] = struct {
				status db.LinkStatus
				msg    string
			}{status: db.LinkStatusApplied, msg: ""}
		} else {
			linkStatusByID[connAck.ConnectionId] = struct {
				status db.LinkStatus
				msg    string
			}{status: db.LinkStatusFailed, msg: connAck.ErrorMsg}
		}
	}

	for _, link := range links {
		if link.SourceNodeID != nodeID || link.Deleted {
			continue
		}
		if status, ok := linkStatusByID[link.LinkID]; ok {
			link.Status = status.status
			link.StatusMsg = status.msg
			if err := s.dbService.UpdateLink(link); err != nil {
				return err
			}
			if status.status == db.LinkStatusApplied {
				routes := s.dbService.GetRoutesByLinkID(link.LinkID)
				for _, route := range routes {
					impactedNodes[route.SourceNodeID] = struct{}{}
				}
			}
			if status.status == db.LinkStatusFailed {
				routes := s.dbService.GetRoutesByLinkID(link.LinkID)
				for _, route := range routes {
					if err := s.dbService.MarkRouteAsFailed(route.ID, status.msg); err != nil {
						return fmt.Errorf("failed to mark route %s as failed after link ack failure: %w", route.String(), err)
					}
				}
			}
		}
	}

	for srcNodeID := range impactedNodes {
		zlog.Debug().Str("source_node_id", srcNodeID).Msg("enqueueing route reconcile after link reconcile")
		s.routeQueue.Add(RouteReconcileRequest{NodeID: srcNodeID})
	}
	return nil
}
