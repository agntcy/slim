package nbapiservice

import (
	"context"
	"fmt"

	"github.com/rs/zerolog"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	commonUtil "github.com/agntcy/slim/control-plane/common/util"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/groupservice"
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
	SaveConnection(
		nodeEntry *controlplaneApi.NodeEntry, connection *controllerapi.Connection,
	) (string, error)
	GetConnectionDetails(nodeID string, connectionID string) (string, error)
	SaveSubscription(nodeID string, subscription *controllerapi.Subscription) (string, error)
	GetSubscription(nodeID string, subscriptionID string) (*controllerapi.Subscription, error)
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
	CreateConnection(
		ctx context.Context,
		nodeEntry *controlplaneApi.NodeEntry,
		connection *controllerapi.Connection,
	) error
	CreateSubscription(
		ctx context.Context,
		nodeEntry *controlplaneApi.NodeEntry,
		subscription *controllerapi.Subscription,
	) error
	DeleteSubscription(
		ctx context.Context,
		nodeEntry *controlplaneApi.NodeEntry,
		subscription *controllerapi.Subscription,
	) error
}

type nbAPIService struct {
	controlplaneApi.UnimplementedControlPlaneServiceServer
	config config.APIConfig

	logConfig    config.LogConfig
	nodeService  *NodeService
	routeService *RouteService

	groupService *groupservice.GroupService
}

func NewNorthboundAPIServer(
	config config.APIConfig,
	logConfig config.LogConfig,
	nodeService *NodeService,
	routeService *RouteService,
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

func (s *nbAPIService) ListSubscriptions(
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

func (s *nbAPIService) CreateConnection(
	ctx context.Context,
	createConnectionRequest *controlplaneApi.CreateConnectionRequest) (
	*controlplaneApi.CreateConnectionResponse, error,
) {
	ctx = util.GetContextWithLogger(ctx, s.logConfig)
	nodeEntry, err := s.nodeService.GetNodeByID(createConnectionRequest.NodeId)
	if err != nil {
		return nil, fmt.Errorf("failed to get node by ID: %w", err)
	}

	err = s.routeService.CreateConnection(ctx, nodeEntry, createConnectionRequest.Connection)
	if err != nil {
		return nil, fmt.Errorf("failed to send config command to node: %w", err)
	}

	connID, err := s.nodeService.SaveConnection(nodeEntry, createConnectionRequest.Connection)
	if err != nil {
		return nil, fmt.Errorf("failed to save connection to db: %w", err)
	}
	return &controlplaneApi.CreateConnectionResponse{
		Success:      true,
		ConnectionId: connID,
	}, nil
}

func (s *nbAPIService) CreateSubscription(
	ctx context.Context,
	createSubscriptionRequest *controlplaneApi.CreateSubscriptionRequest) (
	*controlplaneApi.CreateSubscriptionResponse, error,
) {
	ctx = util.GetContextWithLogger(ctx, s.logConfig)
	zlog := zerolog.Ctx(ctx)
	nodeEntry, err := s.nodeService.GetNodeByID(createSubscriptionRequest.NodeId)
	if err != nil {
		return nil, fmt.Errorf("failed to get node by ID: %w", err)
	}

	connectionID := createSubscriptionRequest.Subscription.ConnectionId
	// Instead of ID node should send endpoint as connection Id to the Node
	endpoint, err := s.nodeService.GetConnectionDetails(createSubscriptionRequest.NodeId, connectionID)
	if err != nil {
		return nil, fmt.Errorf("failed to get connection by ID: %w", err)
	}

	createSubscriptionRequest.Subscription.ConnectionId = endpoint

	err = s.routeService.CreateSubscription(ctx, nodeEntry, createSubscriptionRequest.Subscription)
	if err != nil {
		zlog.Error().Msgf("router error: %v\n", err.Error())

		return nil, fmt.Errorf("failed to create subscription: %w", err)
	}

	// To properly save the subscription we restore the original connection ID making sure that db validation passes
	createSubscriptionRequest.Subscription.ConnectionId = connectionID
	subscriptionID, err := s.nodeService.SaveSubscription(createSubscriptionRequest.NodeId,
		createSubscriptionRequest.Subscription)
	if err != nil {
		zlog.Error().Msgf("save error: %v\n", err.Error())

		return nil, fmt.Errorf("failed to save subscription: %w", err)
	}
	response := &controlplaneApi.CreateSubscriptionResponse{
		Success:        true,
		SubscriptionId: subscriptionID,
	}
	return response, nil
}

func (s *nbAPIService) DeleteSubscription(
	ctx context.Context,
	deleteSubscriptionRequest *controlplaneApi.DeleteSubscriptionRequest) (
	*controlplaneApi.DeleteSubscriptionResponse, error,
) {
	ctx = util.GetContextWithLogger(ctx, s.logConfig)
	nodeEntry, err := s.nodeService.GetNodeByID(deleteSubscriptionRequest.NodeId)
	if err != nil {
		return nil, fmt.Errorf("failed to get node by ID: %w", err)
	}

	subscription, err := s.nodeService.GetSubscription(deleteSubscriptionRequest.NodeId,
		deleteSubscriptionRequest.SubscriptionId)
	if err != nil {
		return nil, fmt.Errorf("failed to get subscription: %w", err)
	}
	connectionID := subscription.ConnectionId
	endpoint, err := s.nodeService.GetConnectionDetails(deleteSubscriptionRequest.NodeId, connectionID)
	if err != nil {
		return nil, fmt.Errorf("failed to get connectiondetails: %w", err)
	}
	subscription.ConnectionId = endpoint

	err = s.routeService.DeleteSubscription(ctx, nodeEntry, subscription)
	if err != nil {
		return nil, fmt.Errorf("failed to delete subscription: %w", err)
	}
	return &controlplaneApi.DeleteSubscriptionResponse{
		Success: true,
	}, nil
}

func (s *nbAPIService) DeregisterNode(
	context.Context,
	*controlplaneApi.Node,
) (*controlplaneApi.DeregisterNodeResponse, error) {
	return &controlplaneApi.DeregisterNodeResponse{
		Success: false,
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
	storedChannel, err := s.groupService.GetChannelDetails(ctx, deleteChannelRequest.ChannelId)
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
	storedChannel, err := s.groupService.GetChannelDetails(ctx, addParticipantRequest.ChannelId)
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
	storedChannel, err := s.groupService.GetChannelDetails(ctx, deleteParticipantRequest.ChannelId)
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
