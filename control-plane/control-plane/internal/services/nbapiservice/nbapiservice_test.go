package nbapiservice

import (
	"context"
	"fmt"
	"testing"

	"github.com/stretchr/testify/assert"
	"google.golang.org/protobuf/types/known/wrapperspb"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/groupservice"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/routes"
)

type mockNodeService struct {
	nodes         []*controlplaneApi.NodeEntry
	listNodesFunc func(
		ctx context.Context, req *controlplaneApi.NodeListRequest) (*controlplaneApi.NodeListResponse, error)
	getNodeByIDFunc    func(nodeID string) (*controlplaneApi.NodeEntry, error)
	saveConnectionFunc func(
		nodeEntry *controlplaneApi.NodeEntry, connection *controllerapi.Connection) (string, error)
	getConnectionDetailsFunc func(nodeID string, connectionID string) (string, error)
	saveSubscriptionFunc     func(nodeID string, subscription *controllerapi.Subscription) (string, error)
	getSubscriptionFunc      func(nodeID string, subscriptionID string) (*controllerapi.Subscription, error)
}

func (m *mockNodeService) ListNodes(
	ctx context.Context,
	req *controlplaneApi.NodeListRequest,
) (*controlplaneApi.NodeListResponse, error) {
	if m.listNodesFunc != nil {
		return m.listNodesFunc(ctx, req)
	}
	return &controlplaneApi.NodeListResponse{Entries: m.nodes}, nil
}

func (m *mockNodeService) GetNodeByID(
	nodeID string,
) (*controlplaneApi.NodeEntry, error) {
	if m.getNodeByIDFunc != nil {
		return m.getNodeByIDFunc(nodeID)
	}
	return nil, nil
}

func (m *mockNodeService) SaveConnection(
	nodeEntry *controlplaneApi.NodeEntry,
	connection *controllerapi.Connection,
) (string, error) {
	if m.saveConnectionFunc != nil {
		return m.saveConnectionFunc(nodeEntry, connection)
	}
	return "", nil
}

func (m *mockNodeService) GetConnectionDetails(
	nodeID string,
	connectionID string,
) (string, error) {
	if m.getConnectionDetailsFunc != nil {
		return m.getConnectionDetailsFunc(nodeID, connectionID)
	}
	return "", nil
}

func (m *mockNodeService) SaveSubscription(
	nodeID string,
	subscription *controllerapi.Subscription,
) (string, error) {
	if m.saveSubscriptionFunc != nil {
		return m.saveSubscriptionFunc(nodeID, subscription)
	}
	return "", nil
}

func (m *mockNodeService) GetSubscription(
	nodeID string,
	subscriptionID string,
) (*controllerapi.Subscription, error) {
	if m.getSubscriptionFunc != nil {
		return m.getSubscriptionFunc(nodeID, subscriptionID)
	}
	return nil, nil
}

type mockRouteService struct {
	subscriptions         map[string][]*controllerapi.SubscriptionEntry
	connections           map[string][]*controllerapi.ConnectionEntry
	listSubscriptionsFunc func(
		ctx context.Context, node *controlplaneApi.NodeEntry) (*controllerapi.SubscriptionListResponse, error)
	listConnectionsFunc func(
		ctx context.Context, node *controlplaneApi.NodeEntry) (*controllerapi.ConnectionListResponse, error)
	createConnectionFunc func(
		ctx context.Context, node *controlplaneApi.NodeEntry, connection *controllerapi.Connection) error
	createSubscriptionFunc func(
		ctx context.Context, node *controlplaneApi.NodeEntry, subscription *controllerapi.Subscription) error
	deleteSubscriptionFunc func(
		ctx context.Context, node *controlplaneApi.NodeEntry, subscription *controllerapi.Subscription) error
}

func (m *mockRouteService) AddRoute(_ context.Context, _ routes.Route) (string, error) {
	return "route-id", nil
}

func (m *mockRouteService) DeleteRoute(_ context.Context, _ routes.Route) error {
	return nil
}

func (m *mockRouteService) ListSubscriptions(
	ctx context.Context,
	node *controlplaneApi.NodeEntry,
) (*controllerapi.SubscriptionListResponse, error) {
	if m.listSubscriptionsFunc != nil {
		return m.listSubscriptionsFunc(ctx, node)
	}
	if subs, ok := m.subscriptions[node.Id]; ok {
		return &controllerapi.SubscriptionListResponse{Entries: subs}, nil
	}
	return nil, fmt.Errorf("no SubscriptionListResponse received")
}

func (m *mockRouteService) ListConnections(
	ctx context.Context,
	node *controlplaneApi.NodeEntry,
) (*controllerapi.ConnectionListResponse, error) {
	if m.listConnectionsFunc != nil {
		return m.listConnectionsFunc(ctx, node)
	}
	if conns, ok := m.connections[node.Id]; ok {
		return &controllerapi.ConnectionListResponse{Entries: conns}, nil
	}
	return nil, fmt.Errorf("no ConnectionListResponse received")
}

func (m *mockRouteService) CreateConnection(
	ctx context.Context,
	node *controlplaneApi.NodeEntry,
	connection *controllerapi.Connection,
) error {
	if m.createConnectionFunc != nil {
		return m.createConnectionFunc(ctx, node, connection)
	}
	return nil
}

func (m *mockRouteService) CreateSubscription(
	ctx context.Context,
	node *controlplaneApi.NodeEntry,
	subscription *controllerapi.Subscription,
) error {
	if m.createSubscriptionFunc != nil {
		return m.createSubscriptionFunc(ctx, node, subscription)
	}
	return nil
}

func (m *mockRouteService) DeleteSubscription(
	ctx context.Context,
	node *controlplaneApi.NodeEntry,
	subscription *controllerapi.Subscription,
) error {
	if m.deleteSubscriptionFunc != nil {
		return m.deleteSubscriptionFunc(ctx, node, subscription)
	}
	return nil
}

func TestGetModeratorNode_FindsMatchingModerator(t *testing.T) {
	ctx := context.Background()
	moderatorRoute := "org/default/alice/0"
	organization := "org"
	namespace := "default"
	agentType := "alice"
	agentID := uint64(0)

	node := &controlplaneApi.NodeEntry{Id: "node1"}
	sub := &controllerapi.SubscriptionEntry{
		Component_0: organization,
		Component_1: namespace,
		Component_2: agentType,
		Id:          wrapperspb.UInt64(agentID),
	}
	nodeService := &mockNodeService{nodes: []*controlplaneApi.NodeEntry{node}}
	routeService := &mockRouteService{
		subscriptions: map[string][]*controllerapi.SubscriptionEntry{
			"node1": {sub},
		},
	}

	s := &nbAPIService{
		config:       config.APIConfig{},
		nodeService:  nodeService,
		routeService: routeService,
		groupService: &groupservice.GroupService{},
	}

	result, err := s.getModeratorNode(ctx, []string{moderatorRoute})
	assert.NoError(t, err)
	assert.Equal(t, node, result)
}

func TestGetModeratorNode_NoMatchingModerator_ReturnsFirstNode(t *testing.T) {
	ctx := context.Background()
	moderatorRoute := "org/default/alice/0"
	node1 := &controlplaneApi.NodeEntry{Id: "node1"}
	node2 := &controlplaneApi.NodeEntry{Id: "node2"}

	nodeService := &mockNodeService{nodes: []*controlplaneApi.NodeEntry{node1, node2}}
	routeService := &mockRouteService{
		subscriptions: map[string][]*controllerapi.SubscriptionEntry{
			"node1": {},
			"node2": {},
		},
	}

	s := &nbAPIService{
		config:       config.APIConfig{},
		nodeService:  nodeService,
		routeService: routeService,
		groupService: &groupservice.GroupService{},
	}

	result, err := s.getModeratorNode(ctx, []string{moderatorRoute})
	assert.NoError(t, err)
	assert.Equal(t, node1, result)
}

func TestGetModeratorNode_NoNodesAvailable(t *testing.T) {
	ctx := context.Background()
	nodeService := &mockNodeService{nodes: []*controlplaneApi.NodeEntry{}}
	routeService := &mockRouteService{subscriptions: map[string][]*controllerapi.SubscriptionEntry{}}

	s := &nbAPIService{
		config:       config.APIConfig{},
		nodeService:  nodeService,
		routeService: routeService,
		groupService: &groupservice.GroupService{},
	}

	_, err := s.getModeratorNode(ctx, []string{"org/default/alice/0"})
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "no nodes available")
}
