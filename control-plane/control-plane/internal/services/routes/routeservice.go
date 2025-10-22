package routes

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"reflect"
	"sync"
	"time"

	"github.com/google/uuid"
	"github.com/rs/zerolog"
	"google.golang.org/protobuf/types/known/wrapperspb"
	"k8s.io/client-go/util/workqueue"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
)

const AllNodesID = "*"

type RouteService struct {
	mu                sync.RWMutex
	queue             workqueue.TypedRateLimitingInterface[RouteReconcileRequest]
	dbService         db.DataAccess
	cmdHandler        nodecontrol.NodeCommandHandler
	reconcilerConfig  config.ReconcilerConfig
	reconcilerThreads []*RouteReconciler
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
	reconcilerConfig config.ReconcilerConfig) *RouteService {
	rateLimiter := workqueue.NewTypedItemExponentialFailureRateLimiter[RouteReconcileRequest](
		5*time.Millisecond, 1000*time.Second)
	queue := workqueue.NewTypedRateLimitingQueue[RouteReconcileRequest](rateLimiter)
	return &RouteService{
		queue:            queue,
		dbService:        dbService,
		cmdHandler:       cmdHandler,
		reconcilerConfig: reconcilerConfig,
	}
}

func (s *RouteService) Start(ctx context.Context) error {
	// start route reconcilers
	zlog := zerolog.Ctx(ctx)
	zlog.Info().Msg("Starting route reconcilers")
	s.reconcilerThreads = make([]*RouteReconciler, s.reconcilerConfig.Threads)
	for i := 0; i < s.reconcilerConfig.Threads; i++ {
		reconciler := NewRouteReconciler(fmt.Sprintf("reconciler-%v", i),
			s.reconcilerConfig.MaxRequeues, s.queue, s.dbService, s.cmdHandler)
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
	routeID, err := s.addSingleRoute(ctx, dbRoute)
	if err != nil {
		return "", fmt.Errorf("error adding route: %w", err)
	}

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
			_, err2 := s.addSingleRoute(ctx, newRoute)
			if err2 != nil {
				return "", fmt.Errorf("error adding route: %w", err2)
			}
		}
	}

	return routeID, nil
}

func (s *RouteService) addSingleRoute(ctx context.Context, dbRoute db.Route) (string, error) {
	if dbRoute.SourceNodeID != AllNodesID {
		endpoint, configData, err := s.getConnectionDetails(dbRoute)
		if err != nil {
			return "", fmt.Errorf("failed to set connection details for route: %w", err)
		}
		dbRoute.DestEndpoint = endpoint
		dbRoute.ConnConfigData = configData
	}

	routeID := s.dbService.AddRoute(dbRoute)
	zerolog.Ctx(ctx).Info().Msgf("Route added: %s", routeID)
	if dbRoute.SourceNodeID != AllNodesID {
		s.queue.Add(RouteReconcileRequest{NodeID: dbRoute.SourceNodeID})
	}
	return routeID, nil
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

	// delete routes for all existing nodes if the route is for all nodes
	if route.SourceNodeID == AllNodesID {
		dbRoute, err := s.dbService.GetRouteForSrcAndDestinationAndName(route.SourceNodeID, route.Component0,
			route.Component1, route.Component2, route.ComponentID, route.DestNodeID, route.DestEndpoint)
		if err != nil {
			return fmt.Errorf("failed to fetch route for delete (%w)", err)
		}
		routeID := dbRoute.GetID()
		err = s.dbService.DeleteRoute(routeID)
		if err != nil {
			return fmt.Errorf("failed to delete route for %s (%w)", routeID, err)
		}
		zerolog.Ctx(ctx).Info().Msgf("Route %s deleted.", routeID)

		routes := s.dbService.GetRoutesForDestinationNodeIDAndName(route.DestNodeID, route.Component0,
			route.Component1, route.Component2, route.ComponentID)
		for _, r := range routes {
			if err := s.deleteSingleRoute(ctx, r.SourceNodeID, r.GetID()); err != nil {
				return err
			}
		}

		return nil
	}

	dbRoute, err := s.dbService.GetRouteForSrcAndDestinationAndName(route.SourceNodeID, route.Component0,
		route.Component1, route.Component2, route.ComponentID, route.DestNodeID, route.DestEndpoint)
	if err != nil {
		return fmt.Errorf("failed to fetch route for delete (%w)", err)
	}
	routeID := dbRoute.GetID()
	if err := s.deleteSingleRoute(ctx, route.SourceNodeID, routeID); err != nil {
		return err
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
func (s *RouteService) NodeRegistered(ctx context.Context, nodeID string, connDetailsUpdated bool) {
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

		endpoint, configData, err := s.getConnectionDetails(newRoute)
		if err != nil {
			zlog.Error().Err(err).Msgf("Failed to get connection details for route: %s", newRoute.GetID())
		}
		newRoute.DestEndpoint = endpoint
		newRoute.ConnConfigData = configData
		routeID := s.dbService.AddRoute(newRoute)
		zlog.Debug().Msgf("generic route created: %s", routeID)
	}
	zlog.Debug().Msgf("routes created: %v", len(genericRoutes))

	if connDetailsUpdated {
		// if connection details were updated, we also need to check routes for other nodes
		// which might be affected by the new node connection details
		zlog.Info().Msgf("Connection details changed, checking routes with DestinationNodeID: %s", nodeID)
		routesToBeChecked := s.dbService.GetRoutesForDestinationNodeID(nodeID)
		for _, r := range routesToBeChecked {

			// get new conn details and compare with existing ones, if they differ, mark existing as deleted
			// and create a new route and reconcile
			endpoint, configData, err := s.getConnectionDetails(r)
			if err != nil {
				zlog.Error().Msgf("failed to get connection details for route %s: %v", r.GetID(), err)
				continue
			}
			if r.DestEndpoint != endpoint || r.ConnConfigData != configData {
				zerolog.Ctx(ctx).Info().Msgf("Mark route for delete: %s", r.GetID())
				err := s.dbService.MarkRouteAsDeleted(r.GetID())
				if err != nil {
					zlog.Error().Msgf("failed to mark route %s as deleted: %v", r.GetID(), err)
					continue
				}
				newRoute := db.Route{
					SourceNodeID:   r.SourceNodeID,
					DestNodeID:     r.DestNodeID,
					DestEndpoint:   endpoint,
					ConnConfigData: configData,
					Component0:     r.Component0,
					Component1:     r.Component1,
					Component2:     r.Component2,
					ComponentID:    r.ComponentID,
					Deleted:        false,
				}
				routeID := s.dbService.AddRoute(newRoute)
				zerolog.Ctx(ctx).Info().Msgf("New route added: %s", routeID)

				s.queue.Add(RouteReconcileRequest{NodeID: r.SourceNodeID})
			}
		}

		zlog.Info().Msgf("Connection details changed, checking routes with SourceNodeID: %s", nodeID)
		routesToBeChecked = s.dbService.GetRoutesForNodeID(nodeID)
		for _, r := range routesToBeChecked {

			// get new conn details and compare with existing ones, if they differ, mark existing as deleted
			// and create a new route and reconcile
			endpoint, configData, err := s.getConnectionDetails(r)
			if err != nil {
				zlog.Error().Msgf("failed to get connection details for route %s: %v", r.GetID(), err)
				continue
			}
			if r.DestEndpoint != endpoint || r.ConnConfigData != configData {
				err := s.dbService.MarkRouteAsDeleted(r.GetID())
				if err != nil {
					zlog.Error().Msgf("failed to mark route %s as deleted: %v", r.GetID(), err)
					continue
				}
				newRoute := db.Route{
					SourceNodeID:   r.SourceNodeID,
					DestNodeID:     r.DestNodeID,
					DestEndpoint:   endpoint,
					ConnConfigData: configData,
					Component0:     r.Component0,
					Component1:     r.Component1,
					Component2:     r.Component2,
					ComponentID:    r.ComponentID,
					Deleted:        false,
				}
				routeID := s.dbService.AddRoute(newRoute)
				zerolog.Ctx(ctx).Info().Msgf("Route changed: %s", routeID)
			}

		}
	}

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

func (s *RouteService) getConnectionDetails(route db.Route) (endpoint string, configData string, err error) {
	if route.DestNodeID == "" {
		return route.DestEndpoint, route.ConnConfigData, nil
	}

	destNode, err := s.dbService.GetNode(route.DestNodeID)
	if err != nil {
		return "", "", fmt.Errorf("failed to fetch destination node %s: %w", route.DestNodeID, err)
	}
	if len(destNode.ConnDetails) == 0 {
		return "", "", fmt.Errorf("no connections found for destination node %s", destNode.ID)
	}
	srcNode, err2 := s.dbService.GetNode(route.SourceNodeID)
	if err2 != nil {
		return "", "", fmt.Errorf("failed to fetch source node %s: %w", route.SourceNodeID, err2)
	}

	connDetails, localConnection := selectConnection(destNode, srcNode)
	connID, configData, err := generateConfigData(connDetails, localConnection)
	if err != nil {
		return "", "", fmt.Errorf("failed to generate config data for route %v: %w", route, err)
	}

	return connID, configData, nil
}

// selectConnection selects the most appropriate connection details from the destination node's connections.
// Returns first connection from source to destination node and true if nodes have the same group name,
// or the first connection with external endpoint specified and false otherwise,
// meaning that externalEndpoint should be used to set up connection from src node.
func selectConnection(dstNode *db.Node, srcNode *db.Node) (db.ConnectionDetails, bool) {
	if dstNode.GroupName == nil && srcNode.GroupName == nil ||
		(dstNode.GroupName != nil && srcNode.GroupName != nil && *dstNode.GroupName == *srcNode.GroupName) {
		// same group, return first connection
		return dstNode.ConnDetails[0], true
	}
	// different groups, return first connection with external endpoint defined
	for _, conn := range dstNode.ConnDetails {
		if conn.ExternalEndpoint != nil && *conn.ExternalEndpoint != "" {
			return conn, false
		}
	}
	// no external endpoint defined, return first connection
	return dstNode.ConnDetails[0], false
}

func generateConfigData(detail db.ConnectionDetails, localConnection bool) (string, string, error) {
	truev := true
	falsev := false
	skipVerify := false
	config := ConnectionConfig{
		Endpoint: detail.Endpoint,
	}
	if !localConnection {
		if detail.ExternalEndpoint == nil || *detail.ExternalEndpoint == "" {
			return "", "", fmt.Errorf("no external endpoint defined for connection %v", detail)
		}
		config.Endpoint = *detail.ExternalEndpoint
	} else {
		skipVerify = true // skip verification for local connections
	}
	if !detail.MTLSRequired {
		config.Endpoint = "http://" + config.Endpoint
		config.TLS = &TLS{Insecure: &truev}
	} else {
		config.Endpoint = "https://" + config.Endpoint
		config.TLS = &TLS{
			Insecure:           &falsev,
			InsecureSkipVerify: &skipVerify,
			CERTFile:           stringPtr("/svids/tls.crt"),
			KeyFile:            stringPtr("/svids/tls.key"),
			CAFile:             stringPtr("/svids/svid_bundle.pem"),
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
		return "", "", fmt.Errorf("failed to encode connection config: %w", err)
	}

	return config.Endpoint, buf.String(), nil
}

func stringPtr(s string) *string {
	return &s
}
