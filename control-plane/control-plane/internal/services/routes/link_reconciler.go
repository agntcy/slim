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
	createConnMap := make(map[string]*controllerapi.Connection)
	deletedLinksByID := make(map[string][]db.Link)

	reconcileRoutesForNodes := map[string]struct{}{}
	// we need to send routes for current node after registration
	reconcileRoutesForNodes[nodeID] = struct{}{}

	for _, link := range links {
		if link.SourceNodeID != nodeID {
			continue
		}

		if link.Deleted {
			deletedLinksByID[link.LinkID] = append(deletedLinksByID[link.LinkID], link)
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
		if _, exists := createConnMap[link.LinkID]; !exists {
			createConnMap[link.LinkID] = &controllerapi.Connection{
				ConnectionId: link.LinkID,
				ConfigData:   configData,
			}
		}
	}

	connectionsToCreate := make([]*controllerapi.Connection, 0, len(createConnMap))
	for _, c := range createConnMap {
		connectionsToCreate = append(connectionsToCreate, c)
	}
	connectionsToDelete := make([]string, 0, len(deletedLinksByID))
	for linkID := range deletedLinksByID {
		connectionsToDelete = append(connectionsToDelete, linkID)
	}

	messageID := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageID,
		Payload: &controllerapi.ControlMessage_ConfigCommand{
			ConfigCommand: &controllerapi.ConfigurationCommand{
				ConnectionsToCreate:   connectionsToCreate,
				SubscriptionsToSet:    []*controllerapi.Subscription{},
				SubscriptionsToDelete: []*controllerapi.Subscription{},
				ConnectionsToDelete:   connectionsToDelete,
			},
		},
	}
	zlog.Info().
		Int("connections_count", len(connectionsToCreate)).
		Int("connections_to_delete_count", len(connectionsToDelete)).
		Int("subscriptions_count", 0).
		Int("subscriptions_to_delete_count", 0).
		Str("message_id", messageID).
		Msg("Sending link configuration command to registered node")
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

	type ackStatus struct {
		success bool
		msg     string
	}
	ackStatusByID := make(map[string]ackStatus)
	createAppliedCount := 0
	createFailedCount := 0
	deleteAckedCount := 0
	deleteFailedCount := 0
	createIDs := make(map[string]struct{}, len(createConnMap))
	for id := range createConnMap {
		createIDs[id] = struct{}{}
	}
	deleteIDs := make(map[string]struct{}, len(deletedLinksByID))
	for id := range deletedLinksByID {
		deleteIDs[id] = struct{}{}
	}
	for _, connAck := range ack.GetConnectionsStatus() {
		ackStatusByID[connAck.ConnectionId] = ackStatus{
			success: connAck.Success,
			msg:     connAck.ErrorMsg,
		}
		if _, ok := createIDs[connAck.ConnectionId]; ok {
			if connAck.Success {
				createAppliedCount++
			} else {
				createFailedCount++
			}
		}
		if _, ok := deleteIDs[connAck.ConnectionId]; ok {
			if connAck.Success {
				deleteAckedCount++
			} else {
				deleteFailedCount++
			}
		}
	}
	zlog.Info().
		Str("original_message_id", ack.OriginalMessageId).
		Int("links_applied_count", createAppliedCount).
		Int("links_failed_count", createFailedCount).
		Int("links_deleted_acked_count", deleteAckedCount).
		Int("links_deleted_failed_count", deleteFailedCount).
		Int("links_total_ack_count", len(ack.GetConnectionsStatus())).
		Msg("Link configuration command processing completed")

	// Apply status for applied links.
	for _, link := range links {
		if link.SourceNodeID != nodeID || link.Deleted {
			continue
		}
		if _, ok := createIDs[link.LinkID]; !ok {
			continue
		}
		if status, ok := ackStatusByID[link.LinkID]; ok {
			if status.success {
				link.Status = db.LinkStatusApplied
				link.StatusMsg = ""
			} else {
				link.Status = db.LinkStatusFailed
				link.StatusMsg = status.msg
			}
			if err := s.dbService.UpdateLink(link); err != nil {
				return err
			}
			if status.success {
				zlog.Info().
					Str("link_id", link.LinkID).
					Str("source_node_id", link.SourceNodeID).
					Str("dest_node_id", link.DestNodeID).
					Msg("Link status updated to APPLIED")
			} else {
				zlog.Info().
					Str("link_id", link.LinkID).
					Str("source_node_id", link.SourceNodeID).
					Str("dest_node_id", link.DestNodeID).
					Str("status_msg", status.msg).
					Msg("Link status updated to FAILED")
			}
			if status.success {
				routes := s.dbService.GetRoutesByLinkID(link.LinkID)
				for _, route := range routes {
					reconcileRoutesForNodes[route.SourceNodeID] = struct{}{}
				}
			}
		}
	}

	// Apply ACK-driven cleanup for deleted links.
	retryDeleteLinks := make([]string, 0)
	for linkID, deletedLinks := range deletedLinksByID {
		status, ok := ackStatusByID[linkID]
		if !ok || !status.success {
			reason := "missing delete ack"
			if ok {
				reason = status.msg
				if reason == "" {
					reason = "delete ack reported failure"
				}
			}
			for _, link := range deletedLinks {
				link.Status = db.LinkStatusFailed
				link.StatusMsg = reason
				if err := s.dbService.UpdateLink(link); err != nil {
					return fmt.Errorf("failed to update deleted link %s after failed delete ack: %w", linkID, err)
				}
			}
			retryDeleteLinks = append(retryDeleteLinks, linkID)
			zlog.Info().
				Str("link_id", linkID).
				Str("status_msg", reason).
				Msg("Delete ack failed or missing; keeping link deleted for retry")
			continue
		}

		for _, link := range deletedLinks {
			if err := s.dbService.DeleteLink(link); err != nil {
				return fmt.Errorf("failed to delete link %s after successful delete ack: %w", linkID, err)
			}
		}
		zlog.Info().
			Str("link_id", linkID).
			Msg("Deleted link records after successful delete ack")
	}

	for srcNodeID := range reconcileRoutesForNodes {
		zlog.Debug().Str("source_node_id", srcNodeID).Msg("enqueueing route reconcile after link reconcile")
		s.routeQueue.Add(RouteReconcileRequest{NodeID: srcNodeID})
	}

	if len(retryDeleteLinks) > 0 {
		return fmt.Errorf("delete ack failed or missing for links: %v", retryDeleteLinks)
	}
	return nil
}
