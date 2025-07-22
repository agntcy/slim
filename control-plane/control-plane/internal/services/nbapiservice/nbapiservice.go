package nbapiservice

import (
	"context"
	"fmt"
	"log"

	"github.com/agntcy/slim/control-plane/common/options"
	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/util"
)

type NorthboundAPIServer interface {
	controlplaneApi.ControlPlaneServiceServer
}

type nbAPIService struct {
	controlplaneApi.UnimplementedControlPlaneServiceServer

	config        config.APIConfig
	nodeService   *nodeService
	routeService  *routeService
	configService *configService
}

func NewNorthboundAPIServer(
	config config.APIConfig,
	nodeService *nodeService,
	routeService *routeService,
	configService *configService,
) NorthboundAPIServer {
	cpServer := &nbAPIService{
		config:        config,
		nodeService:   nodeService,
		routeService:  routeService,
		configService: configService,
	}
	return cpServer
}

func (s *nbAPIService) ListSubscriptions(ctx context.Context, node *controlplaneApi.Node) (*controllerapi.SubscriptionListResponse, error) {
	ctx = util.GetContextWithLogger(ctx, s.config.LogConfig)
	nodeEntry, err := s.nodeService.GetNodeByID(node.Id)
	if err != nil {
		return nil, fmt.Errorf("failed to get node by ID: %v", err)
	}
	return s.routeService.ListSubscriptions(ctx, nodeEntry)
}

func (s *nbAPIService) ListConnections(ctx context.Context, node *controlplaneApi.Node) (*controllerapi.ConnectionListResponse, error) {
	ctx = util.GetContextWithLogger(ctx, s.config.LogConfig)
	nodeEntry, err := s.nodeService.GetNodeByID(node.Id)
	if err != nil {
		return nil, fmt.Errorf("failed to get node by ID: %v", err)
	}
	return s.routeService.ListConnections(ctx, nodeEntry)
}

func (s *nbAPIService) ListNodes(ctx context.Context, nodeListRequest *controlplaneApi.NodeListRequest) (*controlplaneApi.NodeListResponse, error) {
	ctx = util.GetContextWithLogger(ctx, s.config.LogConfig)
	return s.nodeService.ListNodes(ctx, nodeListRequest)
}

func (s *nbAPIService) ModifyConfiguration(ctx context.Context, message *controlplaneApi.ConfigurationCommand) (*controllerapi.Ack, error) {
	ctx = util.GetContextWithLogger(ctx, s.config.LogConfig)
	nodeEntry, err := s.nodeService.GetNodeByID(message.NodeId)
	if err != nil {
		log.Fatalf("failed to get node by ID: %v", err)
	}
	endpoint := fmt.Sprintf("%s:%d", nodeEntry.Host, nodeEntry.Port)

	opts := options.NewOptions()
	opts.Server = endpoint
	opts.TLSInsecure = true
	return s.configService.ModifyConfiguration(ctx, message.ConfigurationCommand, opts)
}

func (s *nbAPIService) CreateConnection(ctx context.Context, createConnectionRequest *controlplaneApi.CreateConnectionRequest) (*controlplaneApi.CreateConnectionResponse, error) {
	ctx = util.GetContextWithLogger(ctx, s.config.LogConfig)
	nodeEntry, err := s.nodeService.GetNodeByID(createConnectionRequest.NodeId)
	if err != nil {
		return nil, fmt.Errorf("failed to get node by ID: %v", err)
	}

	err = s.routeService.CreateConnection(ctx, nodeEntry, createConnectionRequest.Connection)
	if err != nil {
		return nil, fmt.Errorf("failed to send config command to node: %v", err)
	}

	connID, err := s.nodeService.SaveConnection(nodeEntry, createConnectionRequest.Connection)

	return &controlplaneApi.CreateConnectionResponse{
		Success:      true,
		ConnectionId: connID,
	}, nil
}

func (s *nbAPIService) CreateSubscription(ctx context.Context, createSubscriptionRequest *controlplaneApi.CreateSubscriptionRequest) (*controlplaneApi.CreateSubscriptionResponse, error) {
	ctx = util.GetContextWithLogger(ctx, s.config.LogConfig)
	nodeEntry, err := s.nodeService.GetNodeByID(createSubscriptionRequest.NodeId)
	if err != nil {
		return nil, fmt.Errorf("failed to get node by ID: %v", err)
	}

	connectionID := createSubscriptionRequest.Subscription.ConnectionId
	// Instead of ID node should send endpoint as connection Id to the Node
	endpoint, err := s.nodeService.GetConnectionDetails(createSubscriptionRequest.NodeId, connectionID)
	if err != nil {
		return nil, fmt.Errorf("failed to get connection by ID: %v", err)
	}

	createSubscriptionRequest.Subscription.ConnectionId = endpoint

	err = s.routeService.CreateSubscription(ctx, nodeEntry, createSubscriptionRequest.Subscription)
	if err != nil {
		fmt.Printf("router error: %v\n", err.Error())
		return nil, fmt.Errorf("failed to create subscription: %v", err)
	}

	// To properly save the subscription we restore the original connection ID making sure that db validation passes
	createSubscriptionRequest.Subscription.ConnectionId = connectionID
	subscriptionID, err := s.nodeService.SaveSubscription(createSubscriptionRequest.NodeId, createSubscriptionRequest.Subscription)
	if err != nil {
		fmt.Printf("save error: %v\n", err.Error())

		return nil, fmt.Errorf("failed to save subscription: %v", err)
	}
	response := &controlplaneApi.CreateSubscriptionResponse{
		Success:        true,
		SubscriptionId: subscriptionID,
	}
	return response, nil
}

func (s *nbAPIService) DeleteSubscription(ctx context.Context, deleteSubscriptionRequest *controlplaneApi.DeleteSubscriptionRequest) (*controlplaneApi.DeleteSubscriptionResponse, error) {
	ctx = util.GetContextWithLogger(ctx, s.config.LogConfig)
	nodeEntry, err := s.nodeService.GetNodeByID(deleteSubscriptionRequest.NodeId)
	if err != nil {
		return nil, fmt.Errorf("failed to get node by ID: %v", err)
	}

	subscription, err := s.nodeService.GetSubscription(deleteSubscriptionRequest.NodeId, deleteSubscriptionRequest.SubscriptionId)
	if err != nil {
		return nil, fmt.Errorf("failed to get subscription: %v", err)
	}
	connectionID := subscription.ConnectionId
	endpoint, err := s.nodeService.GetConnectionDetails(deleteSubscriptionRequest.NodeId, connectionID)
	if err != nil {
		return nil, fmt.Errorf("failed to get connectiondetails: %v", err)
	}
	subscription.ConnectionId = endpoint

	err = s.routeService.DeleteSubscription(ctx, nodeEntry, subscription)
	if err != nil {
		return nil, fmt.Errorf("failed to delete subscription: %v", err)
	}
	return &controlplaneApi.DeleteSubscriptionResponse{
		Success: true,
	}, nil
}

func (s *nbAPIService) DeregisterNode(context.Context, *controlplaneApi.Node) (*controlplaneApi.DeregisterNodeResponse, error) {
	return &controlplaneApi.DeregisterNodeResponse{
		Success: false,
	}, nil
}
