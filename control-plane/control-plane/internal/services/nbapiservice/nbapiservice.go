package nbapiservice

import (
	"context"
	"encoding/json"
	"fmt"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/groupservice"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/routes"
	"github.com/agntcy/slim/control-plane/control-plane/internal/util"
)

type NorthboundAPIServer interface {
	controlplaneApi.ControlPlaneServiceServer
}

type nbAPIService struct {
	controlplaneApi.UnimplementedControlPlaneServiceServer
	config       config.APIConfig
	logConfig    config.LogConfig
	nodeService  *NodeService
	routeService *routes.RouteService
	groupService *groupservice.GroupService
}

func NewNorthboundAPIServer(
	config config.APIConfig,
	logConfig config.LogConfig,
	nodeService *NodeService,
	routeService *routes.RouteService,
	groupService *groupservice.GroupService,
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
	ctx context.Context, createChannelRequest *controllerapi.CreateChannelRequest) (
	*controllerapi.CreateChannelResponse, error) {
	return s.groupService.CreateChannel(ctx, createChannelRequest)
}

func (s *nbAPIService) DeleteChannel(
	ctx context.Context, deleteChannelRequest *controllerapi.DeleteChannelRequest) (
	*controllerapi.Ack, error) {
	return s.groupService.DeleteChannel(ctx, deleteChannelRequest)
}

func (s *nbAPIService) AddParticipant(
	ctx context.Context, addParticipantRequest *controllerapi.AddParticipantRequest) (
	*controllerapi.Ack, error) {
	return s.groupService.AddParticipant(ctx, addParticipantRequest)
}

func (s *nbAPIService) DeleteParticipant(
	ctx context.Context, deleteParticipantRequest *controllerapi.DeleteParticipantRequest) (
	*controllerapi.Ack, error) {
	return s.groupService.DeleteParticipant(ctx, deleteParticipantRequest)
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
