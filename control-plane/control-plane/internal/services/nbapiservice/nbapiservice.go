package nbapiservice

import (
	"context"
	"encoding/json"
	"fmt"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	commonUtil "github.com/agntcy/slim/control-plane/common/util"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/groupservice"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/routes"
	"github.com/agntcy/slim/control-plane/control-plane/internal/util"
)

type NorthboundAPIServer interface {
	controlplaneApi.ControlPlaneServiceServer
}

type NodeManager interface {
	ListNodes(
		context.Context, *controlplaneApi.NodeListRequest,
	) (*controlplaneApi.NodeListResponse, error)
	GetNodeByID(nodeID string) (*controlplaneApi.NodeEntry, error)
}

type RouteManager interface {
	ListSubscriptions(
		_ context.Context,
		nodeEntry *controlplaneApi.NodeEntry,
	) (*controllerapi.SubscriptionListResponse, error)
	ListConnections(
		_ context.Context,
		nodeEntry *controlplaneApi.NodeEntry,
	) (*controllerapi.ConnectionListResponse, error)

	AddRoute(ctx context.Context, route routes.Route) (string, error)
	DeleteRoute(ctx context.Context, route routes.Route) error
}

type nbAPIService struct {
	controlplaneApi.UnimplementedControlPlaneServiceServer
	config config.APIConfig

	logConfig    config.LogConfig
	nodeService  NodeManager
	routeService RouteManager
	groupService groupservice.GroupManager
}

func NewNorthboundAPIServer(
	config config.APIConfig,
	logConfig config.LogConfig,
	nodeService NodeManager,
	routeService RouteManager,
	groupService groupservice.GroupManager,
) NorthboundAPIServer {
	cpServer := &nbAPIService{
		config:       config,
		logConfig:    logConfig,
		nodeService:  nodeService,
		routeService: routeService,
		groupService: groupService,
	}
	return cpServer
}

func (s *nbAPIService) ListRoutes(
	ctx context.Context,
	node *controlplaneApi.Node,
) (*controllerapi.SubscriptionListResponse, error) {
	ctx = util.GetContextWithLogger(ctx, s.logConfig)
	nodeEntry, err := s.nodeService.GetNodeByID(node.Id)
	if err != nil {
		return nil, fmt.Errorf("failed to get node by ID: %w", err)
	}
	return s.routeService.ListSubscriptions(ctx, nodeEntry)
}

func (s *nbAPIService) ListConnections(
	ctx context.Context,
	node *controlplaneApi.Node,
) (*controllerapi.ConnectionListResponse, error) {
	ctx = util.GetContextWithLogger(ctx, s.logConfig)
	nodeEntry, err := s.nodeService.GetNodeByID(node.Id)
	if err != nil {
		return nil, fmt.Errorf("failed to get node by ID: %w", err)
	}
	return s.routeService.ListConnections(ctx, nodeEntry)
}

func (s *nbAPIService) ListNodes(
	ctx context.Context,
	nodeListRequest *controlplaneApi.NodeListRequest,
) (*controlplaneApi.NodeListResponse, error) {
	ctx = util.GetContextWithLogger(ctx, s.logConfig)
	return s.nodeService.ListNodes(ctx, nodeListRequest)
}

func validateConnection(conn *controllerapi.Connection) error {
	// Parse the JSON config data
	var config map[string]interface{}
	if err := json.Unmarshal([]byte(conn.ConfigData), &config); err != nil {
		return fmt.Errorf("failed to parse config data: %w", err)
	}

	// Extract the endpoint value
	endpoint, exists := config["endpoint"]
	if !exists {
		return fmt.Errorf("endpoint not found in config data")
	}
	endpointStr, ok := endpoint.(string)
	if !ok {
		return fmt.Errorf("endpoint is not a string")
	}
	if endpointStr == "" {
		return fmt.Errorf("endpoint cannot be empty")
	}
	if endpointStr != conn.ConnectionId {
		return fmt.Errorf("endpoint in config data does not match connection ID")
	}
	return nil
}

func (s *nbAPIService) AddRoute(
	ctx context.Context,
	addRouteRequest *controlplaneApi.AddRouteRequest) (
	*controlplaneApi.AddRouteResponse, error,
) {
	ctx = util.GetContextWithLogger(ctx, s.logConfig)

	_, err := s.nodeService.GetNodeByID(addRouteRequest.NodeId)
	if err != nil {
		return nil, fmt.Errorf("invalid source nodeID: %w", err)
	}
	if addRouteRequest.DestNodeId != "" {
		_, err = s.nodeService.GetNodeByID(addRouteRequest.DestNodeId)
		if err != nil {
			return nil, fmt.Errorf("invalid destination nodeID: %w", err)
		}
	} else {
		// if destNodeId is empty, connectionId must be provided
		if addRouteRequest.Connection == nil || addRouteRequest.Connection.ConnectionId == "" {
			return nil, fmt.Errorf("either destNodeId or connectionId must be provided")
		}
		err = validateConnection(addRouteRequest.Connection)
		if err != nil {
			return nil, fmt.Errorf("invalid connection: %w", err)
		}
	}

	route := routes.Route{
		SourceNodeID: addRouteRequest.NodeId,
		DestNodeID:   addRouteRequest.DestNodeId,
		Component0:   addRouteRequest.Subscription.Component_0,
		Component1:   addRouteRequest.Subscription.Component_1,
		Component2:   addRouteRequest.Subscription.Component_2,
		ComponentID:  addRouteRequest.Subscription.Id,
	}
	if addRouteRequest.Subscription.ConnectionId != "" && addRouteRequest.Connection != nil {
		route.DestEndpoint = addRouteRequest.Connection.ConnectionId
		route.ConnConfigData = addRouteRequest.Connection.ConfigData
	}
	routeID, err := s.routeService.AddRoute(ctx, route)
	if err != nil {
		return nil, fmt.Errorf("failed to add route: %w", err)
	}

	response := &controlplaneApi.AddRouteResponse{
		Success: true,
		RouteId: routeID,
	}
	return response, nil
}

func (s *nbAPIService) DeleteRoute(
	ctx context.Context,
	deleteRouteRequest *controlplaneApi.DeleteRouteRequest) (
	*controlplaneApi.DeleteRouteResponse, error,
) {
	ctx = util.GetContextWithLogger(ctx, s.logConfig)

	_, err := s.nodeService.GetNodeByID(deleteRouteRequest.NodeId)
	if err != nil {
		return nil, fmt.Errorf("invalid source nodeID: %w", err)
	}

	if deleteRouteRequest.DestNodeId != "" {
		_, err = s.nodeService.GetNodeByID(deleteRouteRequest.DestNodeId)
		if err != nil {
			return nil, fmt.Errorf("invalid destination nodeID: %w", err)
		}
	} else if deleteRouteRequest.Subscription.ConnectionId == "" {
		return nil, fmt.Errorf("either destNodeId or connectionId must be provided")
	}

	route := routes.Route{
		SourceNodeID: deleteRouteRequest.NodeId,
		DestNodeID:   deleteRouteRequest.DestNodeId,
		Component0:   deleteRouteRequest.Subscription.Component_0,
		Component1:   deleteRouteRequest.Subscription.Component_1,
		Component2:   deleteRouteRequest.Subscription.Component_2,
		ComponentID:  deleteRouteRequest.Subscription.Id,
	}
	if deleteRouteRequest.Subscription.ConnectionId != "" {
		route.DestEndpoint = deleteRouteRequest.Subscription.ConnectionId
	}

	err = s.routeService.DeleteRoute(ctx, route)
	if err != nil {
		return nil, fmt.Errorf("failed to delete route: %w", err)
	}
	return &controlplaneApi.DeleteRouteResponse{
		Success: true,
	}, nil
}

func (s *nbAPIService) CreateChannel(
	ctx context.Context, createChannelRequest *controlplaneApi.CreateChannelRequest) (
	*controlplaneApi.CreateChannelResponse, error) {
	node, err := s.getModeratorNode(ctx, createChannelRequest.Moderators)
	if err != nil {
		return nil, fmt.Errorf("failed to get available node for channel creation: %w", err)
	}
	return s.groupService.CreateChannel(ctx, createChannelRequest, node)
}

func (s *nbAPIService) DeleteChannel(
	ctx context.Context, deleteChannelRequest *controllerapi.DeleteChannelRequest) (
	*controllerapi.Ack, error) {
	storedChannel, err := s.groupService.GetChannelDetails(ctx, deleteChannelRequest.ChannelName)
	if err != nil {
		return nil, fmt.Errorf("failed to get channel: %w", err)
	}
	node, err := s.getModeratorNode(ctx, storedChannel.Moderators)
	if err != nil {
		return nil, fmt.Errorf("failed to get available node for channel deletion: %w", err)
	}
	return s.groupService.DeleteChannel(ctx, deleteChannelRequest, node)
}

func (s *nbAPIService) AddParticipant(
	ctx context.Context, addParticipantRequest *controllerapi.AddParticipantRequest) (
	*controllerapi.Ack, error) {
	storedChannel, err := s.groupService.GetChannelDetails(ctx, addParticipantRequest.ChannelName)
	if err != nil {
		return nil, fmt.Errorf("failed to get channel: %w", err)
	}
	node, err := s.getModeratorNode(ctx, storedChannel.Moderators)
	if err != nil {
		return nil, fmt.Errorf("failed to get available node for adding participant: %w", err)
	}
	return s.groupService.AddParticipant(ctx, addParticipantRequest, node)
}

func (s *nbAPIService) DeleteParticipant(
	ctx context.Context, deleteParticipantRequest *controllerapi.DeleteParticipantRequest) (
	*controllerapi.Ack, error) {
	storedChannel, err := s.groupService.GetChannelDetails(ctx, deleteParticipantRequest.ChannelName)
	if err != nil {
		return nil, fmt.Errorf("failed to get channel: %w", err)
	}
	node, err := s.getModeratorNode(ctx, storedChannel.Moderators)
	if err != nil {
		return nil, fmt.Errorf("failed to get available node for deleting participant: %w", err)
	}
	return s.groupService.DeleteParticipant(ctx, deleteParticipantRequest, node)
}

func (s *nbAPIService) ListChannels(
	ctx context.Context, listChannelsRequest *controllerapi.ListChannelsRequest) (
	*controllerapi.ListChannelsResponse, error) {
	return s.groupService.ListChannels(ctx, listChannelsRequest)
}

func (s *nbAPIService) ListParticipants(
	ctx context.Context, listParticipantsRequest *controllerapi.ListParticipantsRequest) (
	*controllerapi.ListParticipantsResponse, error) {
	return s.groupService.ListParticipants(ctx, listParticipantsRequest)
}

func (s *nbAPIService) getModeratorNode(ctx context.Context, moderators []string) (
	*controlplaneApi.NodeEntry, error) {
	moderatorToFind := moderators[0]
	organization, namespace, agentType, _, err := commonUtil.ParseRoute(moderatorToFind)
	if err != nil {
		return nil, fmt.Errorf("failed to parse moderator route: %w", err)
	}
	nodeListResponse, err := s.ListNodes(ctx, &controlplaneApi.NodeListRequest{})
	if err != nil {
		return nil, fmt.Errorf("failed to list nodes: %w", err)
	}
	if nodeListResponse.GetEntries() == nil || len(nodeListResponse.GetEntries()) == 0 {
		return nil, fmt.Errorf("no nodes available")
	}

	nodes := nodeListResponse.GetEntries()

	for _, node := range nodes {
		subscriptionList, err := s.routeService.ListSubscriptions(ctx, node)
		if err != nil {
			continue
		}
		subscriptionEntries := subscriptionList.GetEntries()
		for _, subscriptionEntry := range subscriptionEntries {
			if isSubscriptionSameAsModerator(subscriptionEntry, organization, namespace, agentType) {
				return node, nil
			}
		}
	}

	return nodes[0], nil
}

func isSubscriptionSameAsModerator(
	subscription *controllerapi.SubscriptionEntry,
	organization, namespace, agentType string,
) bool {
	return subscription.Component_0 == organization &&
		subscription.Component_1 == namespace &&
		subscription.Component_2 == agentType
}
