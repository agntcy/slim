package nbapiservice

import (
	"context"
	"reflect"

	"github.com/stretchr/testify/mock"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/routes"
)

type mockNodeService struct {
	mock.Mock
}

func (m *mockNodeService) ListNodes(
	ctx context.Context,
	req *controlplaneApi.NodeListRequest,
) (*controlplaneApi.NodeListResponse, error) {
	args := m.Called(ctx, req)
	if args.Get(0) == nil {
		return nil, args.Error(1)
	}
	return args.Get(0).(*controlplaneApi.NodeListResponse), args.Error(1)
}

func (m *mockNodeService) GetNodeByID(
	nodeID string,
) (*controlplaneApi.NodeEntry, error) {
	args := m.Called(nodeID)
	if args.Get(0) == nil {
		return nil, args.Error(1)
	}
	return args.Get(0).(*controlplaneApi.NodeEntry), args.Error(1)
}

func (m *mockNodeService) SaveConnection(
	nodeEntry *controlplaneApi.NodeEntry,
	connection *controllerapi.Connection,
) (string, error) {
	args := m.Called(nodeEntry, connection)
	return args.String(0), args.Error(1)
}

func (m *mockNodeService) GetConnectionDetails(
	nodeID string,
	connectionID string,
) (string, error) {
	args := m.Called(nodeID, connectionID)
	return args.String(0), args.Error(1)
}

func (m *mockNodeService) SaveSubscription(
	nodeID string,
	subscription *controllerapi.Subscription,
) (string, error) {
	args := m.Called(nodeID, subscription)
	return args.String(0), args.Error(1)
}

func (m *mockNodeService) GetSubscription(
	nodeID string,
	subscriptionID string,
) (*controllerapi.Subscription, error) {
	args := m.Called(nodeID, subscriptionID)
	if args.Get(0) == nil {
		return nil, args.Error(1)
	}
	return args.Get(0).(*controllerapi.Subscription), args.Error(1)
}

type mockRouteService struct {
	mock.Mock
}

func (m *mockRouteService) ListRoutes(ctx context.Context,
	request *controlplaneApi.RouteListRequest) (*controlplaneApi.RouteListResponse, error) {

	args := m.Called(ctx, request)
	return args.Get(0).(*controlplaneApi.RouteListResponse), args.Error(1)
}

func (m *mockRouteService) AddRoute(ctx context.Context, route routes.Route) (string, error) {
	args := m.Called(ctx, route)
	return args.String(0), args.Error(1)
}

func (m *mockRouteService) DeleteRoute(ctx context.Context, route routes.Route) error {
	args := m.Called(ctx, route)
	return args.Error(0)
}

func (m *mockRouteService) ListSubscriptions(
	ctx context.Context,
	node *controlplaneApi.NodeEntry,
) (*controllerapi.SubscriptionListResponse, error) {
	args := m.Called(ctx, node)
	if args.Get(0) == nil {
		return nil, args.Error(1)
	}
	return args.Get(0).(*controllerapi.SubscriptionListResponse), args.Error(1)
}

func (m *mockRouteService) ListConnections(
	ctx context.Context,
	node *controlplaneApi.NodeEntry,
) (*controllerapi.ConnectionListResponse, error) {
	args := m.Called(ctx, node)
	if args.Get(0) == nil {
		return nil, args.Error(1)
	}
	return args.Get(0).(*controllerapi.ConnectionListResponse), args.Error(1)
}

func (m *mockRouteService) CreateConnection(
	ctx context.Context,
	node *controlplaneApi.NodeEntry,
	connection *controllerapi.Connection,
) error {
	args := m.Called(ctx, node, connection)
	return args.Error(0)
}

func (m *mockRouteService) CreateSubscription(
	ctx context.Context,
	node *controlplaneApi.NodeEntry,
	subscription *controllerapi.Subscription,
) error {
	args := m.Called(ctx, node, subscription)
	return args.Error(0)
}

func (m *mockRouteService) DeleteSubscription(
	ctx context.Context,
	node *controlplaneApi.NodeEntry,
	subscription *controllerapi.Subscription,
) error {
	args := m.Called(ctx, node, subscription)
	return args.Error(0)
}

type mockDataAccess struct {
	mock.Mock
}

func (m *mockDataAccess) FilterRoutesBySourceAndDestination(sourceNodeID string, destNodeID string) []db.Route {
	args := m.Called(sourceNodeID, destNodeID)
	return args.Get(0).([]db.Route)
}

// AddRoute implements db.DataAccess.
func (m *mockDataAccess) AddRoute(route db.Route) string {
	args := m.Called(route)
	return args.String(0)
}

// DeleteChannel implements db.DataAccess.
func (m *mockDataAccess) DeleteChannel(channelID string) error {
	args := m.Called(channelID)
	return args.Error(0)
}

// DeleteNode implements db.DataAccess.
func (m *mockDataAccess) DeleteNode(id string) error {
	args := m.Called(id)
	return args.Error(0)
}

// DeleteRoute implements db.DataAccess.
func (m *mockDataAccess) DeleteRoute(routeID string) error {
	args := m.Called(routeID)
	return args.Error(0)
}

// GetChannel implements db.DataAccess.
func (m *mockDataAccess) GetChannel(channelID string) (db.Channel, error) {
	args := m.Called(channelID)
	if args.Get(0) == nil {
		return db.Channel{}, args.Error(1)
	}
	return args.Get(0).(db.Channel), args.Error(1)
}

// GetRouteByID implements db.DataAccess.
func (m *mockDataAccess) GetRouteByID(routeID string) *db.Route {
	args := m.Called(routeID)
	if args.Get(0) == nil {
		return nil
	}
	return args.Get(0).(*db.Route)
}

// GetRoutesForNodeID implements db.DataAccess.
func (m *mockDataAccess) GetRoutesForNodeID(nodeID string) []db.Route {
	args := m.Called(nodeID)
	if args.Get(0) == nil {
		return nil
	}
	return args.Get(0).([]db.Route)
}

// ListChannels implements db.DataAccess.
func (m *mockDataAccess) ListChannels() ([]db.Channel, error) {
	args := m.Called()
	if args.Get(0) == nil {
		return nil, args.Error(1)
	}
	return args.Get(0).([]db.Channel), args.Error(1)
}

// MarkRouteAsDeleted implements db.DataAccess.
func (m *mockDataAccess) MarkRouteAsDeleted(routeID string) error {
	args := m.Called(routeID)
	return args.Error(0)
}

// SaveChannel implements db.DataAccess.
func (m *mockDataAccess) SaveChannel(channelID string, moderators []string) error {
	args := m.Called(channelID, moderators)
	return args.Error(0)
}

// SaveNode implements db.DataAccess.
func (m *mockDataAccess) SaveNode(node db.Node) (string, error) {
	args := m.Called(node)
	return args.String(0), args.Error(1)
}

// UpdateChannel implements db.DataAccess.
func (m *mockDataAccess) UpdateChannel(channel db.Channel) error {
	args := m.Called(channel)
	return args.Error(0)
}

func (m *mockDataAccess) GetNode(nodeID string) (*db.Node, error) {
	args := m.Called(nodeID)
	if args.Get(0) == nil {
		return nil, args.Error(1)
	}
	return args.Get(0).(*db.Node), args.Error(1)
}

func (m *mockDataAccess) ListNodes() []db.Node {
	args := m.Called()
	return args.Get(0).([]db.Node)
}

type mockNodeCommandHandler struct {
	mock.Mock
}

// AddStream implements nodecontrol.NodeCommandHandler.
func (m *mockNodeCommandHandler) AddStream(
	ctx context.Context, nodeID string, stream controllerapi.ControllerService_OpenControlChannelServer) {
	m.Called(ctx, nodeID, stream)
}

// GetConnectionStatus implements nodecontrol.NodeCommandHandler.
func (m *mockNodeCommandHandler) GetConnectionStatus(
	ctx context.Context, nodeID string) (nodecontrol.NodeStatus, error) {
	args := m.Called(ctx, nodeID)
	return args.Get(0).(nodecontrol.NodeStatus), args.Error(1)
}

// RemoveStream implements nodecontrol.NodeCommandHandler.
func (m *mockNodeCommandHandler) RemoveStream(
	ctx context.Context, nodeID string) error {
	args := m.Called(ctx, nodeID)
	return args.Error(0)
}

// ResponseReceived implements nodecontrol.NodeCommandHandler.
func (m *mockNodeCommandHandler) ResponseReceived(
	ctx context.Context, nodeID string, command *controllerapi.ControlMessage) {
	m.Called(ctx, nodeID, command)
}

// SendMessage implements nodecontrol.NodeCommandHandler.
func (m *mockNodeCommandHandler) SendMessage(
	ctx context.Context, nodeID string, configurationCommand *controllerapi.ControlMessage) error {
	args := m.Called(ctx, nodeID, configurationCommand)
	return args.Error(0)
}

// UpdateConnectionStatus implements nodecontrol.NodeCommandHandler.
func (m *mockNodeCommandHandler) UpdateConnectionStatus(
	ctx context.Context, nodeID string, status nodecontrol.NodeStatus) {
	m.Called(ctx, nodeID, status)
}

// WaitForResponse implements nodecontrol.NodeCommandHandler.
func (m *mockNodeCommandHandler) WaitForResponse(
	ctx context.Context, nodeID string, messageType reflect.Type, messageID string) (
	*controllerapi.ControlMessage, error) {
	args := m.Called(ctx, nodeID, messageType, messageID)
	if args.Get(0) == nil {
		return nil, args.Error(1)
	}
	return args.Get(0).(*controllerapi.ControlMessage), args.Error(1)
}

type mockGroupService struct {
	mock.Mock
}

func (m *mockGroupService) CreateChannel(
	ctx context.Context, req *controlplaneApi.CreateChannelRequest) (
	*controlplaneApi.CreateChannelResponse, error) {
	args := m.Called(ctx, req)
	if args.Get(0) == nil {
		return nil, args.Error(1)
	}
	return args.Get(0).(*controlplaneApi.CreateChannelResponse), args.Error(1)
}

func (m *mockGroupService) DeleteChannel(
	ctx context.Context, req *controllerapi.DeleteChannelRequest) (
	*controllerapi.Ack, error) {
	args := m.Called(ctx, req)
	if args.Get(0) == nil {
		return nil, args.Error(1)
	}
	return args.Get(0).(*controllerapi.Ack), args.Error(1)
}

func (m *mockGroupService) AddParticipant(
	ctx context.Context, req *controllerapi.AddParticipantRequest) (
	*controllerapi.Ack, error) {
	args := m.Called(ctx, req)
	if args.Get(0) == nil {
		return nil, args.Error(1)
	}
	return args.Get(0).(*controllerapi.Ack), args.Error(1)
}

func (m *mockGroupService) DeleteParticipant(
	ctx context.Context, req *controllerapi.DeleteParticipantRequest) (
	*controllerapi.Ack, error) {
	args := m.Called(ctx, req)
	if args.Get(0) == nil {
		return nil, args.Error(1)
	}
	return args.Get(0).(*controllerapi.Ack), args.Error(1)
}

func (m *mockGroupService) ListChannels(
	ctx context.Context, req *controllerapi.ListChannelsRequest) (
	*controllerapi.ListChannelsResponse, error) {
	args := m.Called(ctx, req)
	if args.Get(0) == nil {
		return nil, args.Error(1)
	}
	return args.Get(0).(*controllerapi.ListChannelsResponse), args.Error(1)
}

func (m *mockGroupService) ListParticipants(
	ctx context.Context, req *controllerapi.ListParticipantsRequest) (
	*controllerapi.ListParticipantsResponse, error) {
	args := m.Called(ctx, req)
	if args.Get(0) == nil {
		return nil, args.Error(1)
	}
	return args.Get(0).(*controllerapi.ListParticipantsResponse), args.Error(1)
}

func (m *mockGroupService) GetChannelDetails(ctx context.Context, channelName string) (db.Channel, error) {
	args := m.Called(ctx, channelName)
	if args.Get(0) == nil {
		return db.Channel{}, args.Error(1)
	}
	return args.Get(0).(db.Channel), args.Error(1)
}
