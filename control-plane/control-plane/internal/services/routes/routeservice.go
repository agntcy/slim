package routes

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"reflect"
	"sort"
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
	mu               sync.RWMutex
	queue            workqueue.TypedRateLimitingInterface[RouteReconcileRequest]
	linkQueue        workqueue.TypedRateLimitingInterface[LinkReconcileRequest]
	dbService        db.DataAccess
	cmdHandler       nodecontrol.NodeCommandHandler
	reconcilerConfig config.ReconcilerConfig
	reconcilerThread *RouteReconciler
	linkThread       *LinkReconciler
}

type Route struct {
	SourceNodeID string
	DestNodeID   string
	LinkID       string
	Component0   string
	Component1   string
	Component2   string
	ComponentID  *wrapperspb.UInt64Value
}

func NewRouteService(dbService db.DataAccess, cmdHandler nodecontrol.NodeCommandHandler,
	reconcilerConfig config.ReconcilerConfig) *RouteService {
	rateLimiter := workqueue.NewTypedItemExponentialFailureRateLimiter[RouteReconcileRequest](
		5*time.Millisecond, 1000*time.Second)
	queueConfig := workqueue.TypedRateLimitingQueueConfig[RouteReconcileRequest]{
		Name:          "RouteReconcileQueue",
		DelayingQueue: workqueue.NewTypedDelayingQueue[RouteReconcileRequest](),
	}
	queue := workqueue.NewTypedRateLimitingQueueWithConfig[RouteReconcileRequest](rateLimiter, queueConfig)
	linkQueue := workqueue.NewTypedRateLimitingQueueWithConfig[LinkReconcileRequest](
		workqueue.NewTypedItemExponentialFailureRateLimiter[LinkReconcileRequest](5*time.Millisecond, 1000*time.Second),
		workqueue.TypedRateLimitingQueueConfig[LinkReconcileRequest]{
			Name:          "LinkReconcileQueue",
			DelayingQueue: workqueue.NewTypedDelayingQueue[LinkReconcileRequest](),
		},
	)
	return &RouteService{
		queue:            queue,
		linkQueue:        linkQueue,
		dbService:        dbService,
		cmdHandler:       cmdHandler,
		reconcilerConfig: reconcilerConfig,
	}
}

func (s *RouteService) Start(ctx context.Context) error {
	reconciler := NewRouteReconciler("route-reconciler",
		s.reconcilerConfig, s.queue, s.dbService, s.cmdHandler)
	linkReconciler := NewLinkReconciler("link-reconciler",
		s.reconcilerConfig, s.linkQueue, s.queue, s.dbService, s.cmdHandler)
	go func(r *RouteReconciler) {
		r.Run(ctx)
	}(reconciler)
	go func(r *LinkReconciler) {
		r.Run(ctx)
	}(linkReconciler)
	s.reconcilerThread = reconciler
	s.linkThread = linkReconciler
	return nil
}

func (s *RouteService) Stop() {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.linkQueue.ShutDownWithDrain()
	s.queue.ShutDownWithDrain()
}

func (s *RouteService) AddRoute(ctx context.Context, route Route) (string, error) {
	if route.SourceNodeID == "" {
		return "", fmt.Errorf("source node ID cannot be empty")
	}

	if route.DestNodeID == "" {
		return "", fmt.Errorf("destination node ID cannot be empty")
	}

	// source node ID and dest node ID cannot be the same
	if route.SourceNodeID == route.DestNodeID {
		return "", fmt.Errorf("destination node ID cannot be the same as source node ID")
	}

	s.mu.Lock()
	defer s.mu.Unlock()

	dbRoute := db.Route{
		SourceNodeID: route.SourceNodeID,
		DestNodeID:   route.DestNodeID,
		Component0:   route.Component0,
		Component1:   route.Component1,
		Component2:   route.Component2,
		ComponentID:  route.ComponentID,
		Status:       db.RouteStatusPending,
		Deleted:      false,
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
				SourceNodeID: n.ID,
				DestNodeID:   route.DestNodeID,
				Component0:   route.Component0,
				Component1:   route.Component1,
				Component2:   route.Component2,
				ComponentID:  route.ComponentID,
				Status:       db.RouteStatusPending,
				Deleted:      false,
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
		linkID, err := s.findMatchingLinkForRoute(dbRoute)
		if err != nil {
			return "", fmt.Errorf("failed to find matching link for route: %w", err)
		}
		dbRoute.LinkID = linkID
	}

	route, err := s.dbService.AddRoute(dbRoute)
	if err != nil {
		return "", fmt.Errorf("failed to add route to database: %w", err)
	}
	zerolog.Ctx(ctx).Info().Msgf("Route added: %s", route)
	if dbRoute.SourceNodeID != AllNodesID {
		s.queue.Add(RouteReconcileRequest{NodeID: dbRoute.SourceNodeID})
	}
	return route.String(), nil
}

func (s *RouteService) findMatchingLinkForRoute(dbRoute db.Route) (string, error) {
	link, err := s.dbService.FindLinkBetweenNodes(dbRoute.SourceNodeID, dbRoute.DestNodeID)
	if err != nil {
		return "", err
	}
	if link != nil && !link.Deleted {
		return link.LinkID, nil
	}
	return "", fmt.Errorf("no matching link found for source=%s destination=%s",
		dbRoute.SourceNodeID, dbRoute.DestNodeID)
}

func (s *RouteService) DeleteRoute(ctx context.Context, route Route) error {
	s.mu.Lock()
	defer s.mu.Unlock()

	// validate that destination node is set
	if route.DestNodeID == "" {
		return fmt.Errorf("destNodeID must be set")
	}

	// delete routes for all existing nodes if the route is for all nodes
	if route.SourceNodeID == AllNodesID {
		dbRoute, err := s.dbService.GetRouteForSrcAndDestinationAndName(route.SourceNodeID, route.Component0,
			route.Component1, route.Component2, route.ComponentID, route.DestNodeID, route.LinkID)
		if err != nil {
			return fmt.Errorf("failed to fetch route for delete (%w)", err)
		}
		err = s.dbService.DeleteRoute(dbRoute.ID)
		if err != nil {
			return fmt.Errorf("failed to delete route for %s (%w)", dbRoute, err)
		}
		zerolog.Ctx(ctx).Info().Msgf("Route %s deleted.", dbRoute)

		routes := s.dbService.GetRoutesForDestinationNodeIDAndName(route.DestNodeID, route.Component0,
			route.Component1, route.Component2, route.ComponentID)
		for _, r := range routes {
			if err := s.deleteSingleRoute(ctx, r.SourceNodeID, r.ID, r.String()); err != nil {
				return err
			}
		}

		return nil
	}

	if route.LinkID == "" {
		matchRoute := db.Route{
			SourceNodeID: route.SourceNodeID,
			DestNodeID:   route.DestNodeID,
		}
		linkID, err := s.findMatchingLinkForRoute(matchRoute)
		if err != nil {
			return fmt.Errorf("failed to find matching link for route delete: %w", err)
		}
		route.LinkID = linkID
	}

	dbRoute, err := s.dbService.GetRouteForSrcAndDestinationAndName(route.SourceNodeID, route.Component0,
		route.Component1, route.Component2, route.ComponentID, route.DestNodeID, route.LinkID)
	if err != nil {
		return fmt.Errorf("failed to fetch route for delete (%w)", err)
	}
	if err := s.deleteSingleRoute(ctx, route.SourceNodeID, dbRoute.ID, dbRoute.String()); err != nil {
		return err
	}

	return nil
}

func (s *RouteService) deleteSingleRoute(ctx context.Context, nodeID string, routeID uint64, routeKey string) error {
	err := s.dbService.MarkRouteAsDeleted(routeID)
	if err != nil {
		return fmt.Errorf("failed to mark route for delete %s (%w)", routeKey, err)
	}
	zerolog.Ctx(ctx).Info().Msgf("Route marked for delete: %s", routeKey)
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

	reconcileLinkForNodes := map[string]struct{}{}
	if connDetailsUpdated {
		for _, nid := range s.reconnectExistingLinks(ctx, nodeID) {
			reconcileLinkForNodes[nid] = struct{}{}
		}
	}
	for _, nid := range s.ensureLinksForNode(ctx, nodeID) {
		reconcileLinkForNodes[nid] = struct{}{}
	}
	s.ensureRoutesForNode(ctx, nodeID)
	// we need to send links for current node after registration
	reconcileLinkForNodes[nodeID] = struct{}{}
	for nid := range reconcileLinkForNodes {
		s.linkQueue.Add(LinkReconcileRequest{NodeID: nid})
	}

}

func (s *RouteService) reconnectExistingLinks(ctx context.Context, nodeID string) []string {
	zlog := zerolog.Ctx(ctx).With().Str("service", "RouteService").Str("node_id", nodeID).Logger()
	affected := map[string]struct{}{nodeID: {}}
	for _, link := range s.dbService.GetLinksForNode(nodeID) {
		if link.Deleted {
			continue
		}

		endpoint, configData, err := s.getConnectionDetails(ctx, db.Route{
			SourceNodeID: link.SourceNodeID,
			DestNodeID:   link.DestNodeID,
		})
		if err != nil {
			zlog.Error().
				Err(err).
				Str("old_link_id", link.LinkID).
				Str("source_node_id", link.SourceNodeID).
				Str("dest_node_id", link.DestNodeID).
				Msg("failed to compute replacement connection details")
			continue
		}

		replacement, err := s.dbService.AddLink(db.Link{
			LinkID:         uuid.NewString(),
			SourceNodeID:   link.SourceNodeID,
			DestNodeID:     link.DestNodeID,
			DestEndpoint:   endpoint,
			ConnConfigData: configData,
			Status:         db.LinkStatusPending,
			Deleted:        false,
		})
		if err != nil {
			zlog.Error().
				Err(err).
				Str("old_link_id", link.LinkID).
				Str("source_node_id", link.SourceNodeID).
				Str("dest_node_id", link.DestNodeID).
				Msg("failed to create replacement link")
			continue
		}

		zlog.Info().
			Str("old_link_id", link.LinkID).
			Str("new_link_id", replacement.LinkID).
			Str("source_node_id", link.SourceNodeID).
			Str("dest_node_id", link.DestNodeID).
			Str("dest_endpoint", endpoint).
			Msg("Replacement link created")
		affected[replacement.SourceNodeID] = struct{}{}

		for _, r := range s.dbService.GetRoutesByLinkID(link.LinkID) {
			if err := s.dbService.RepointRoute(
				r.ID,
				replacement.LinkID,
				db.RouteStatusPending,
				"waiting for replacement link apply",
			); err != nil {
				zlog.Error().
					Err(err).
					Str("route", r.String()).
					Str("old_link_id", link.LinkID).
					Str("new_link_id", replacement.LinkID).
					Msg("failed to repoint dependent route to replacement link")
				continue
			}
			zlog.Info().
				Uint64("route_id", r.ID).
				Str("source_node_id", r.SourceNodeID).
				Str("old_link_id", link.LinkID).
				Str("new_link_id", replacement.LinkID).
				Msg("Dependent route moved to replacement link and marked pending")
			affected[r.SourceNodeID] = struct{}{}
		}

		link.Deleted = true
		link.StatusMsg = "marked deleted after replacement link creation"
		if err := s.dbService.UpdateLink(link); err != nil {
			zlog.Error().Err(err).Msgf("failed to mark old link %s deleted", link.LinkID)
			continue
		}
		zlog.Info().
			Str("link_id", link.LinkID).
			Str("replacement_link_id", replacement.LinkID).
			Str("source_node_id", link.SourceNodeID).
			Str("dest_node_id", link.DestNodeID).
			Str("reason", link.StatusMsg).
			Msg("Old link marked for delete")
	}
	out := make([]string, 0, len(affected))
	for nid := range affected {
		out = append(out, nid)
	}
	return out
}

func (s *RouteService) ensureLinksForNode(ctx context.Context, nodeID string) []string {
	zlog := zerolog.Ctx(ctx).With().Str("service", "RouteService").Str("node_id", nodeID).Logger()
	srcNode, err := s.dbService.GetNode(nodeID)
	if err != nil {
		zlog.Error().Err(err).Msg("failed to load registered node")
		return []string{nodeID}
	}
	affected := map[string]struct{}{nodeID: {}}
	for _, other := range s.dbService.ListNodes() {
		if other.ID == nodeID {
			continue
		}

		if existing, err := s.dbService.FindLinkBetweenNodes(nodeID, other.ID); err == nil &&
			existing != nil && !existing.Deleted {
			zerolog.Ctx(ctx).Info().
				Str("link", existing.String()).
				Msg("Link already exists, skipping creation")
			continue
		}

		sameGroup := (srcNode.GroupName == nil && other.GroupName == nil) ||
			(srcNode.GroupName != nil && other.GroupName != nil && *srcNode.GroupName == *other.GroupName)
		// make direct link in case of same group, which means direct connectivity between nodes
		if sameGroup {
			if createdSource, ok := s.ensureDirectLink(ctx, nodeID, other.ID); ok {
				affected[createdSource] = struct{}{}
			}
			continue
		}
		// make link to external endpoint if destination node has external endpoint
		hasDstExternal := false
		for _, d := range other.ConnDetails {
			if d.ExternalEndpoint != nil && *d.ExternalEndpoint != "" {
				hasDstExternal = true
				break
			}
		}
		if hasDstExternal {
			if createdSource, ok := s.ensureGroupLink(ctx, nodeID, other.ID); ok {
				affected[createdSource] = struct{}{}
			}
			continue
		}

		// make reverse link to src node external endpoint if source node has external endpoint
		hasSrcExternal := false
		for _, d := range srcNode.ConnDetails {
			if d.ExternalEndpoint != nil && *d.ExternalEndpoint != "" {
				hasSrcExternal = true
				break
			}
		}
		if hasSrcExternal {
			if createdSource, ok := s.ensureGroupLink(ctx, other.ID, nodeID); ok {
				affected[createdSource] = struct{}{}
			}
			continue
		}

		zlog.Error().Msgf("cannot create link between %s and %s: no external endpoint available", nodeID, other.ID)
	}
	out := make([]string, 0, len(affected))
	for nid := range affected {
		out = append(out, nid)
	}
	return out
}

func (s *RouteService) ensureDirectLink(ctx context.Context, sourceNodeID string, destNodeID string) (string, bool) {
	tmpRoute := db.Route{SourceNodeID: sourceNodeID, DestNodeID: destNodeID}
	endpoint, configData, err := s.getConnectionDetails(ctx, tmpRoute)
	if err != nil {
		zerolog.Ctx(ctx).Error().Err(err).Msgf("failed to compute link connection details for %s -> %s",
			sourceNodeID, destNodeID)
		return sourceNodeID, false
	}
	linkID := uuid.NewString()
	_, err = s.dbService.AddLink(db.Link{
		LinkID:         linkID,
		SourceNodeID:   sourceNodeID,
		DestNodeID:     destNodeID,
		DestEndpoint:   endpoint,
		ConnConfigData: configData,
		Status:         db.LinkStatusPending,
		Deleted:        false,
	})
	if err != nil {
		zerolog.Ctx(ctx).Error().Err(err).Msgf("failed to add link %s -> %s", sourceNodeID, destNodeID)
		return sourceNodeID, false
	}
	zerolog.Ctx(ctx).Info().
		Str("link_id", linkID).
		Str("source_node_id", sourceNodeID).
		Str("dest_node_id", destNodeID).
		Str("dest_endpoint", endpoint).
		Msg("Link created")
	return sourceNodeID, true
}

func (s *RouteService) ensureGroupLink(ctx context.Context, sourceNodeID string, destNodeID string) (string, bool) {
	tmpRoute := db.Route{SourceNodeID: sourceNodeID, DestNodeID: destNodeID}
	endpoint, configData, err := s.getConnectionDetails(ctx, tmpRoute)
	if err != nil {
		zerolog.Ctx(ctx).Error().Err(err).Msgf("failed to compute link connection details for %s -> %s",
			sourceNodeID, destNodeID)
		return sourceNodeID, false
	}
	// Avoid duplicate links to the same destination group and endpoint
	// by reusing an existing link when available.
	reused, err := s.dbService.GetLinkForSourceAndEndpoint(sourceNodeID, endpoint)
	if err != nil {
		zerolog.Ctx(ctx).Error().Err(err).Msgf("failed to find reusable link for %s endpoint=%s", sourceNodeID, endpoint)
		return sourceNodeID, false
	}
	linkID := uuid.NewString()
	if reused != nil {
		linkID = reused.LinkID
	}
	_, err = s.dbService.AddLink(db.Link{
		LinkID:         linkID,
		SourceNodeID:   sourceNodeID,
		DestNodeID:     destNodeID,
		DestEndpoint:   endpoint,
		ConnConfigData: configData,
		Status:         db.LinkStatusPending,
		Deleted:        false,
	})
	if err != nil {
		zerolog.Ctx(ctx).Error().Err(err).Msgf("failed to add link %s -> %s", sourceNodeID, destNodeID)
		return sourceNodeID, false
	}
	zerolog.Ctx(ctx).Info().
		Str("link_id", linkID).
		Str("source_node_id", sourceNodeID).
		Str("dest_node_id", destNodeID).
		Str("dest_endpoint", endpoint).
		Msg("Link created")
	return sourceNodeID, true
}

func (s *RouteService) ensureRoutesForNode(ctx context.Context, nodeID string) {
	zlog := zerolog.Ctx(ctx).With().Str("service", "RouteService").Str("node_id", nodeID).Logger()
	genericRoutes := s.dbService.GetRoutesForNodeID(AllNodesID)
	for _, r := range genericRoutes {
		if r.DestNodeID == nodeID {
			continue
		}
		newRoute := db.Route{
			SourceNodeID: nodeID,
			DestNodeID:   r.DestNodeID,
			Component0:   r.Component0,
			Component1:   r.Component1,
			Component2:   r.Component2,
			ComponentID:  r.ComponentID,
			Status:       db.RouteStatusPending,
			Deleted:      false,
		}
		linkID, err := s.findMatchingLinkForRoute(newRoute)
		if err != nil {
			zlog.Error().Err(err).Msgf("failed to find matching link for generic route %s", newRoute)
			continue
		}
		newRoute.LinkID = linkID
		if _, err := s.dbService.AddRoute(newRoute); err != nil {
			zlog.Debug().Err(err).Msgf("generic route already exists or cannot be added: %s", newRoute)
		} else {
			zlog.Info().Msgf("generic route created: %s", newRoute)
		}
	}
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

func (s *RouteService) getConnectionDetails(ctx context.Context,
	route db.Route) (endpoint string, configData string, err error) {
	if route.DestNodeID == "" {
		return "", "", fmt.Errorf("destination node ID cannot be empty")
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
	connID, configData, err := generateConfigData(ctx, connDetails, localConnection, destNode, srcNode)
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

func getSrcNodeSpireSocketPath(srcNode *db.Node) *string {
	for _, conn := range srcNode.ConnDetails {
		if conn.TLSConfig != nil && conn.TLSConfig.Source != nil && conn.TLSConfig.Source.SocketPath != nil {
			return conn.TLSConfig.Source.SocketPath
		}
	}
	return nil
}

func generateConfigData(ctx context.Context, detail db.ConnectionDetails, localConnection bool,
	destNode *db.Node, srcNode *db.Node) (string, string, error) {
	zlog := zerolog.Ctx(ctx)
	truev := true
	falsev := false
	skipVerify := false
	config := db.ClientConnectionConfig{
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
		config.TLS = &db.TLS{Insecure: &truev}
	} else {
		// Socket path for SPIRE should be set in the source node's connection details
		srcNodeSpireSocketPath := getSrcNodeSpireSocketPath(srcNode)
		if srcNodeSpireSocketPath == nil {
			return "", "", fmt.Errorf("no SPIRE socket path found for source node %s", srcNode.ID)
		}
		zlog.Debug().Msgf("SPIRE socket path for source node: %s", *srcNodeSpireSocketPath)

		config.Endpoint = "https://" + config.Endpoint
		config.TLS = &db.TLS{
			Insecure:           &falsev,
			InsecureSkipVerify: &skipVerify,
			Source: &db.TLSSource{
				Type:       "spire",
				SocketPath: srcNodeSpireSocketPath,
			},
			CaSource: &db.CaSource{
				Type:       "spire",
				SocketPath: srcNodeSpireSocketPath,
			},
		}
		if detail.TrustDomain != nil {
			config.TLS.CaSource.TrustDomains = &[]string{*detail.TrustDomain}
			zlog.Debug().Msgf("Trust domain set to: %s", *detail.TrustDomain)
		} else if destNode.GroupName != nil {
			config.TLS.CaSource.TrustDomains = &[]string{*destNode.GroupName}
			zlog.Debug().Msgf("Trust domain set to: %s", *destNode.GroupName)
		}
	}
	var bufferSize int64 = 1024
	config.BufferSize = &bufferSize
	gzip := db.Gzip
	config.Compression = &gzip
	config.ConnectTimeout = stringPtr("10s")
	config.Headers = map[string]string{
		"x-custom-header": "value",
	}

	config.Keepalive = &db.KeepaliveClass{
		HTTP2Keepalive:     stringPtr("2h"),
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
		return "", "", fmt.Errorf("failed to encode connection config: %w", err)
	}
	fmt.Println("Generated connection config:")
	fmt.Println(buf.String())
	return config.Endpoint, buf.String(), nil
}

func stringPtr(s string) *string {
	return &s
}

func (s *RouteService) ListRoutes(_ context.Context,
	request *controlplaneApi.RouteListRequest) (*controlplaneApi.RouteListResponse, error) {

	allRoutes := s.dbService.FilterRoutesBySourceAndDestination(request.GetSrcNodeId(), request.GetDestNodeId())

	// Sort routes: first by wildcard srcNodeID (*), then by srcNodeID
	sort.Slice(allRoutes, func(i, j int) bool {
		srcI := allRoutes[i].SourceNodeID
		srcJ := allRoutes[j].SourceNodeID

		// Check if either is AllNodesID ("*")
		if srcI == AllNodesID && srcJ != AllNodesID {
			return true
		}
		if srcI != AllNodesID && srcJ == AllNodesID {
			return false
		}

		// If both are AllNodesID or both are not, sort by SourceNodeID
		return srcI < srcJ
	})

	routeEntries := make([]*controlplaneApi.RouteEntry, 0, len(allRoutes))
	for _, r := range allRoutes {
		entry := &controlplaneApi.RouteEntry{
			Id:           r.ID,
			SourceNodeId: r.SourceNodeID,
			DestNodeId:   r.DestNodeID,
			Component_0:  r.Component0,
			Component_1:  r.Component1,
			Component_2:  r.Component2,
			StatusMsg:    r.StatusMsg,
			Deleted:      r.Deleted,
			LastUpdated:  r.LastUpdated.Unix(),
		}

		// Set component_id if present
		if r.ComponentID != nil {
			entry.ComponentId = &r.ComponentID.Value
		}

		// Map status
		switch r.Status {
		case db.RouteStatusApplied:
			entry.Status = controlplaneApi.RouteStatus_ROUTE_STATUS_APPLIED
		case db.RouteStatusFailed:
			entry.Status = controlplaneApi.RouteStatus_ROUTE_STATUS_FAILED
		case db.RouteStatusPending:
			entry.Status = controlplaneApi.RouteStatus_ROUTE_STATUS_PENDING
		default:
			entry.Status = controlplaneApi.RouteStatus_ROUTE_STATUS_UNSPECIFIED
		}

		routeEntries = append(routeEntries, entry)
	}

	return &controlplaneApi.RouteListResponse{
		Routes: routeEntries,
	}, nil

}
