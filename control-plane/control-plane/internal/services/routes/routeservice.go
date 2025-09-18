package routes

import (
	"context"
	"fmt"
	"reflect"
	"sync"

	"github.com/google/uuid"
	"github.com/rs/zerolog"
	"google.golang.org/protobuf/types/known/wrapperspb"
	"k8s.io/client-go/util/workqueue"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
)

const AllNodesID = "*"

type RouteService struct {
	mu                   sync.RWMutex
	queue                *workqueue.Typed[RouteReconcileRequest]
	dbService            db.DataAccess
	cmdHandler           nodecontrol.NodeCommandHandler
	reconcilerThreadsNum int
	reconcilerThreads    []*RouteReconciler
}

type Route struct {
	SourceNodeID string
	// if DestNodeID is empty, DestEndpoint should be used to determine the destination
	DestNodeID   string
	DestEndpoint string
	// ConnConfigData is a JSON string containing connection configuration details in case DestEndpoint is set
	ConnConfigData string
	Component0     string
	Component1     string
	Component2     string
	ComponentID    *wrapperspb.UInt64Value
}

func NewRouteService(dbService db.DataAccess, cmdHandler nodecontrol.NodeCommandHandler,
	reconcilerThreadsNum int) *RouteService {
	return &RouteService{
		queue:                workqueue.NewTyped[RouteReconcileRequest](),
		dbService:            dbService,
		cmdHandler:           cmdHandler,
		reconcilerThreadsNum: reconcilerThreadsNum,
	}
}

func (s *RouteService) Start(ctx context.Context) error {
	// start route reconcilers
	zlog := zerolog.Ctx(ctx)
	zlog.Info().Msg("Starting route reconcilers")
	s.reconcilerThreads = make([]*RouteReconciler, s.reconcilerThreadsNum)
	for i := 0; i < s.reconcilerThreadsNum; i++ {
		reconciler := NewRouteReconciler(fmt.Sprintf("reconciler-%v", i), s.queue, s.dbService, s.cmdHandler)
		s.reconcilerThreads[i] = reconciler
		go func(r *RouteReconciler) {
			r.Run(ctx)
		}(reconciler)
	}
	return nil
}

func (s *RouteService) Stop() {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.queue.ShutDownWithDrain()
}

func (s *RouteService) AddRoute(ctx context.Context, route Route) (string, error) {
	if route.SourceNodeID == "" {
		return "", fmt.Errorf("source node ID cannot be empty")
	}

	// validate that either DestNodeID or DestEndpoint and ConnConfigData is set
	if route.DestNodeID == "" {
		if route.DestEndpoint == "" || route.ConnConfigData == "" {
			return "", fmt.Errorf("either destNodeID or both destEndpoint and connConfigData must be set")
		}
	} else {
		// source node ID and dest node ID cannot be the same
		if route.SourceNodeID == route.DestNodeID {
			return "", fmt.Errorf("destination node ID cannot be the same as source node ID")
		}
	}

	s.mu.Lock()
	defer s.mu.Unlock()

	dbRoute := db.Route{
		SourceNodeID:   route.SourceNodeID,
		DestNodeID:     route.DestNodeID,
		DestEndpoint:   route.DestEndpoint,
		Component0:     route.Component0,
		Component1:     route.Component1,
		Component2:     route.Component2,
		ComponentID:    route.ComponentID,
		ConnConfigData: route.ConnConfigData,
		Deleted:        false,
	}

	routeID := s.addSingleRoute(ctx, dbRoute)

	// create routes for all existing nodes if the route is for all nodes
	if route.SourceNodeID == AllNodesID {
		allNodes := s.dbService.ListNodes()
		for _, n := range allNodes {
			if n.ID == route.DestNodeID {
				// skip inserting route for the destination node itself
				continue
			}
			newRoute := db.Route{
				SourceNodeID:   n.ID,
				DestNodeID:     route.DestNodeID,
				DestEndpoint:   route.DestEndpoint,
				Component0:     route.Component0,
				Component1:     route.Component1,
				Component2:     route.Component2,
				ComponentID:    route.ComponentID,
				ConnConfigData: route.ConnConfigData,
				Deleted:        false,
			}
			s.addSingleRoute(ctx, newRoute)
		}
	}

	return routeID, nil
}

func (s *RouteService) addSingleRoute(ctx context.Context, dbRoute db.Route) string {
	routeID := s.dbService.AddRoute(dbRoute)
	zerolog.Ctx(ctx).Info().Msgf("Route added: %s", routeID)
	if dbRoute.SourceNodeID != AllNodesID {
		s.queue.Add(RouteReconcileRequest{NodeID: dbRoute.SourceNodeID})
	}
	return routeID
}

func (s *RouteService) DeleteRoute(ctx context.Context, route Route) error {
	s.mu.Lock()
	defer s.mu.Unlock()

	// validate that either DestNodeID or DestEndpoint is set
	if route.DestNodeID == "" {
		if route.DestEndpoint == "" {
			return fmt.Errorf("either destNodeID or both destEndpoint must be set")
		}
	}

	dbRoute := db.Route{
		SourceNodeID:   route.SourceNodeID,
		DestNodeID:     route.DestNodeID,
		DestEndpoint:   route.DestEndpoint,
		Component0:     route.Component0,
		Component1:     route.Component1,
		Component2:     route.Component2,
		ComponentID:    route.ComponentID,
		ConnConfigData: route.ConnConfigData,
	}
	routeID := dbRoute.GetID()
	if err := s.deleteSingleRoute(ctx, route.SourceNodeID, routeID); err != nil {
		return err
	}

	// delete routes for all existing nodes if the route is for all nodes
	if route.SourceNodeID == AllNodesID {
		allNodes := s.dbService.ListNodes()
		for _, n := range allNodes {
			if n.ID == route.DestNodeID {
				// skip deleting route for the destination node itself
				continue
			}
			newRoute := db.Route{
				SourceNodeID:   n.ID,
				DestNodeID:     route.DestNodeID,
				DestEndpoint:   route.DestEndpoint,
				Component0:     route.Component0,
				Component1:     route.Component1,
				Component2:     route.Component2,
				ComponentID:    route.ComponentID,
				ConnConfigData: route.ConnConfigData,
			}
			newRouteID := newRoute.GetID()

			if err := s.deleteSingleRoute(ctx, n.ID, newRouteID); err != nil {
				return err
			}
		}
	}
	return nil
}

func (s *RouteService) deleteSingleRoute(ctx context.Context, nodeID, routeID string) error {
	err := s.dbService.MarkRouteAsDeleted(routeID)
	if err != nil {
		return fmt.Errorf("failed to mark route for delete %s (%w)", routeID, err)
	}
	zerolog.Ctx(ctx).Info().Msgf("Route marked for delete: %s", routeID)
	if nodeID != AllNodesID {
		s.queue.Add(RouteReconcileRequest{NodeID: nodeID})
	}
	return nil
}

// NodeRegistered should be called when a new node registers to the control plane.
// This will create initial set of routes and trigger reconciliation for the newly registered node.
func (s *RouteService) NodeRegistered(ctx context.Context, nodeID string) {
	s.mu.Lock()
	defer s.mu.Unlock()

	zlog := zerolog.Ctx(ctx).With().Str("service", "RouteService").Str("node_id", nodeID).Logger()
	zlog.Info().Msgf("Create generic routes for node")

	// create generic routes for the newly registered node
	genericRoutes := s.dbService.GetRoutesForNodeID(AllNodesID)
	for _, r := range genericRoutes {
		if r.DestNodeID == nodeID {
			// skip inserting route for the destination node itself
			continue
		}
		newRoute := db.Route{
			SourceNodeID:   nodeID,
			DestNodeID:     r.DestNodeID,
			Component0:     r.Component0,
			Component1:     r.Component1,
			Component2:     r.Component2,
			ComponentID:    r.ComponentID,
			ConnConfigData: r.ConnConfigData,
			Deleted:        false,
		}
		routeID := s.dbService.AddRoute(newRoute)
		zlog.Debug().Msgf("generic route created: %s", routeID)
	}
	zlog.Debug().Msgf("routes created: %v", len(genericRoutes))

	// reconcile generic routes for the newly registered node
	s.queue.Add(RouteReconcileRequest{NodeID: nodeID})
}

func (s *RouteService) ListSubscriptions(
	ctx context.Context,
	nodeEntry *controlplaneApi.NodeEntry,
) (*controllerapi.SubscriptionListResponse, error) {
	zlog := zerolog.Ctx(ctx)
	messageID := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageID,
		Payload:   &controllerapi.ControlMessage_SubscriptionListRequest{},
	}
	if err := s.cmdHandler.SendMessage(ctx, nodeEntry.Id, msg); err != nil {
		return nil, fmt.Errorf("failed to send message: %w", err)
	}
	resp, err := s.cmdHandler.WaitForResponse(ctx, nodeEntry.Id,
		reflect.TypeOf(&controllerapi.ControlMessage_SubscriptionListResponse{}), messageID)
	if err != nil {
		return nil, fmt.Errorf("failed to receive SubscriptionListResponse: %w", err)
	}

	if listResp := resp.GetSubscriptionListResponse(); listResp != nil {
		for _, e := range listResp.Entries {
			var localNames, remoteNames []string
			for _, lc := range e.GetLocalConnections() {
				localNames = append(localNames,
					fmt.Sprintf("local:%d:%s", lc.GetId(), lc.GetConfigData()))
			}
			for _, rc := range e.GetRemoteConnections() {
				remoteNames = append(remoteNames,
					fmt.Sprintf("remote:%d:%s", rc.GetId(), rc.GetConfigData()))
			}

			zlog.Debug().Msgf("%s/%s/%s id=%d local=%v remote=%v\n",
				e.GetComponent_0(), e.GetComponent_1(), e.GetComponent_2(),
				e.GetId().GetValue(),
				localNames, remoteNames,
			)
		}
		return listResp, nil
	}
	return nil, fmt.Errorf("no SubscriptionListResponse received")
}

func (s *RouteService) ListConnections(
	ctx context.Context,
	nodeEntry *controlplaneApi.NodeEntry,
) (*controllerapi.ConnectionListResponse, error) {
	zlog := zerolog.Ctx(ctx)
	messageID := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageID,
		Payload:   &controllerapi.ControlMessage_ConnectionListRequest{},
	}

	if err := s.cmdHandler.SendMessage(ctx, nodeEntry.Id, msg); err != nil {
		return nil, fmt.Errorf("failed to send message: %w", err)
	}
	response, err := s.cmdHandler.WaitForResponse(ctx, nodeEntry.Id,
		reflect.TypeOf(&controllerapi.ControlMessage_ConnectionListResponse{}), messageID)
	if err != nil {
		return nil, fmt.Errorf("failed to receive ConnectionListResponse: %w", err)
	}
	if listResp := response.GetConnectionListResponse(); listResp != nil {
		for _, e := range listResp.Entries {
			zlog.Debug().Msgf("id=%d %s\n", e.GetId(), e.GetConfigData())
		}
		return listResp, nil
	}
	// If we reach here, it means we didn't find a ConnectionListResponse
	return nil, fmt.Errorf("no ConnectionListResponse received")
}
