package nbapiservice

import (
	"context"
	"errors"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/mock"
	"google.golang.org/protobuf/types/known/wrapperspb"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/groupservice"
)

func TestGetModeratorNode_FindsMatchingModerator(t *testing.T) {
	// Arrange
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

	nodeListResp := &controlplaneApi.NodeListResponse{
		Entries: []*controlplaneApi.NodeEntry{node},
	}
	subscriptionListResp := &controllerapi.SubscriptionListResponse{
		Entries: []*controllerapi.SubscriptionEntry{sub},
	}

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)

	mockNodeService.
		On("ListNodes", mock.Anything, mock.AnythingOfType("*controlplanev1.NodeListRequest")).
		Return(nodeListResp, nil)
	mockRouteService.On("ListSubscriptions", mock.Anything, node).Return(subscriptionListResp, nil)

	s := &nbAPIService{
		config:       config.APIConfig{},
		nodeService:  mockNodeService,
		routeService: mockRouteService,
		groupService: &groupservice.GroupService{},
	}

	// Act
	result, err := s.getModeratorNode(context.Background(), []string{moderatorRoute})

	// Assert
	assert.NoError(t, err)
	assert.Equal(t, node, result)

	mockNodeService.AssertExpectations(t)
	mockRouteService.AssertExpectations(t)
}

func TestGetModeratorNode_NoMatchingModerator_ReturnsFirstNode(t *testing.T) {
	// Arrange
	moderatorRoute := "org/default/alice/0"
	node1 := &controlplaneApi.NodeEntry{Id: "node1"}
	node2 := &controlplaneApi.NodeEntry{Id: "node2"}

	nodeListResp := &controlplaneApi.NodeListResponse{
		Entries: []*controlplaneApi.NodeEntry{node1, node2},
	}
	subscriptionListResp1 := &controllerapi.SubscriptionListResponse{
		Entries: []*controllerapi.SubscriptionEntry{},
	}
	subscriptionListResp2 := &controllerapi.SubscriptionListResponse{
		Entries: []*controllerapi.SubscriptionEntry{},
	}

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)

	mockNodeService.
		On("ListNodes", mock.Anything, mock.AnythingOfType("*controlplanev1.NodeListRequest")).
		Return(nodeListResp, nil)
	mockRouteService.On("ListSubscriptions", mock.Anything, node1).Return(subscriptionListResp1, nil)
	mockRouteService.On("ListSubscriptions", mock.Anything, node2).Return(subscriptionListResp2, nil)

	s := &nbAPIService{
		config:       config.APIConfig{},
		nodeService:  mockNodeService,
		routeService: mockRouteService,
		groupService: &groupservice.GroupService{},
	}

	// Act
	result, err := s.getModeratorNode(context.Background(), []string{moderatorRoute})

	// Assert
	assert.NoError(t, err)
	assert.Equal(t, node1, result)

	mockNodeService.AssertExpectations(t)
	mockRouteService.AssertExpectations(t)
}

func TestGetModeratorNode_NoNodesAvailable(t *testing.T) {
	// Arrange
	nodeListResp := &controlplaneApi.NodeListResponse{
		Entries: []*controlplaneApi.NodeEntry{},
	}

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)

	mockNodeService.
		On("ListNodes", mock.Anything, mock.AnythingOfType("*controlplanev1.NodeListRequest")).
		Return(nodeListResp, nil)

	s := &nbAPIService{
		config:       config.APIConfig{},
		nodeService:  mockNodeService,
		routeService: mockRouteService,
		groupService: &groupservice.GroupService{},
	}

	// Act
	_, err := s.getModeratorNode(context.Background(), []string{"org/default/alice/0"})

	// Assert
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "no nodes available")

	mockNodeService.AssertExpectations(t)
}

func TestGetModeratorNode_NodeServiceError(t *testing.T) {
	// Arrange
	expectedError := errors.New("node service error")

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)

	mockNodeService.
		On("ListNodes", mock.Anything, mock.AnythingOfType("*controlplanev1.NodeListRequest")).
		Return(nil, expectedError)

	s := &nbAPIService{
		config:       config.APIConfig{},
		nodeService:  mockNodeService,
		routeService: mockRouteService,
		groupService: &groupservice.GroupService{},
	}

	// Act
	_, err := s.getModeratorNode(context.Background(), []string{"org/default/alice/0"})

	// Assert
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "node service error")

	mockNodeService.AssertExpectations(t)
}

// TODO: re-add if we want to handle route service errors in the getModeratorNode function
/* func TestGetModeratorNode_RouteServiceError(t *testing.T) {
	// Arrange
	node := &controlplaneApi.NodeEntry{Id: "node1"}
	nodeListResp := &controlplaneApi.NodeListResponse{
		Entries: []*controlplaneApi.NodeEntry{node},
	}
	expectedError := errors.New("route service error")

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)

	mockNodeService.
		On("ListNodes", mock.Anything, mock.AnythingOfType("*controlplanev1.NodeListRequest")).
		Return(nodeListResp, nil)
	mockRouteService.On("ListSubscriptions", mock.Anything, node).Return(nil, expectedError)

	s := &nbAPIService{
		config:       config.APIConfig{},
		nodeService:  mockNodeService,
		routeService: mockRouteService,
		groupService: &groupservice.GroupService{},
	}

	// Act
	_, err := s.getModeratorNode(context.Background(), []string{"org/default/alice/0"})

	// Assert
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "route service error")

	mockNodeService.AssertExpectations(t)
	mockRouteService.AssertExpectations(t)
} */

// TestListRoutes_Success tests the successful retrieval of routes for a node.
func TestListSubscriptions_Success(t *testing.T) {
	// Arrange
	nodeID := "node1"
	node := &controlplaneApi.Node{Id: nodeID}
	nodeEntry := &controlplaneApi.NodeEntry{Id: nodeID}
	expectedResponse := &controllerapi.SubscriptionListResponse{
		Entries: []*controllerapi.SubscriptionEntry{},
	}

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)

	mockNodeService.On("GetNodeByID", nodeID).Return(nodeEntry, nil)
	mockRouteService.On("ListSubscriptions", mock.Anything, nodeEntry).Return(expectedResponse, nil)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		routeService: mockRouteService,
	}

	// Act
	result, err := s.ListSubscriptions(context.Background(), node)

	// Assert
	assert.NoError(t, err)
	assert.Equal(t, expectedResponse, result)
	mockNodeService.AssertExpectations(t)
	mockRouteService.AssertExpectations(t)
}

func TestListSubscriptions_NodeNotFound(t *testing.T) {
	nodeID := "nonexistent"
	node := &controlplaneApi.Node{Id: nodeID}
	expectedError := errors.New("node not found")

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)

	mockNodeService.On("GetNodeByID", nodeID).Return(nil, expectedError)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		routeService: mockRouteService,
	}

	// Act
	result, err := s.ListSubscriptions(context.Background(), node)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	assert.Contains(t, err.Error(), "failed to get node by ID")
	mockNodeService.AssertExpectations(t)
}

// TestListConnections_Success tests the successful retrieval of connections for a node.
func TestListConnections_Success(t *testing.T) {
	// Arrange
	nodeID := "node1"
	node := &controlplaneApi.Node{Id: nodeID}
	nodeEntry := &controlplaneApi.NodeEntry{Id: nodeID}
	expectedResponse := &controllerapi.ConnectionListResponse{
		Entries: []*controllerapi.ConnectionEntry{},
	}

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)

	mockNodeService.On("GetNodeByID", nodeID).Return(nodeEntry, nil)
	mockRouteService.On("ListConnections", mock.Anything, nodeEntry).Return(expectedResponse, nil)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		routeService: mockRouteService,
	}

	// Act
	result, err := s.ListConnections(context.Background(), node)

	// Assert
	assert.NoError(t, err)
	assert.Equal(t, expectedResponse, result)
	mockNodeService.AssertExpectations(t)
	mockRouteService.AssertExpectations(t)
}

// ListConnections_NodeNotFound tests the scenario where the node is not found.
func TestListConnections_NodeNotFound(t *testing.T) {
	// Arrange
	nodeID := "nonexistent"
	node := &controlplaneApi.Node{Id: nodeID}
	expectedError := errors.New("node not found")

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)

	mockNodeService.On("GetNodeByID", nodeID).Return(nil, expectedError)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		routeService: mockRouteService,
	}

	// Act
	result, err := s.ListConnections(context.Background(), node)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	assert.Contains(t, err.Error(), "failed to get node by ID")
	mockNodeService.AssertExpectations(t)
}

// TestListNodes_Success tests the successful retrieval of nodes.
func TestListNodes_Success(t *testing.T) {
	// Arrange
	request := &controlplaneApi.NodeListRequest{}
	expectedResponse := &controlplaneApi.NodeListResponse{
		Entries: []*controlplaneApi.NodeEntry{},
	}

	mockNodeService := new(mockNodeService)
	mockNodeService.On("ListNodes", mock.Anything, request).Return(expectedResponse, nil)

	s := &nbAPIService{
		nodeService: mockNodeService,
	}

	// Act
	result, err := s.ListNodes(context.Background(), request)

	// Assert
	assert.NoError(t, err)
	assert.Equal(t, expectedResponse, result)
	mockNodeService.AssertExpectations(t)
}

// TestListNodes_Error tests the scenario where the NodeService returns an error.
func TestListNodes_Error(t *testing.T) {
	// Arrange
	request := &controlplaneApi.NodeListRequest{}
	expectedError := errors.New("service error")

	mockNodeService := new(mockNodeService)
	mockNodeService.On("ListNodes", mock.Anything, request).Return(nil, expectedError)

	s := &nbAPIService{
		nodeService: mockNodeService,
	}

	// Act
	result, err := s.ListNodes(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	assert.Equal(t, expectedError, err)
	mockNodeService.AssertExpectations(t)
}

// TestValidateConnection_Success tests the successful validation of a connection.
func TestValidateConnection_Success(t *testing.T) {
	// Arrange
	conn := &controllerapi.Connection{
		ConnectionId: "test-endpoint",
		ConfigData:   `{"endpoint": "test-endpoint"}`,
	}

	// Act
	err := validateConnection(conn)

	// Assert
	assert.NoError(t, err)
}

// TestValidateConnection_InvalidJSON tests the scenario where the ConfigData is not valid JSON.
func TestValidateConnection_InvalidJSON(t *testing.T) {
	// Arrange
	conn := &controllerapi.Connection{
		ConnectionId: "test-endpoint",
		ConfigData:   `invalid json`,
	}

	// Act
	err := validateConnection(conn)

	// Assert
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "failed to parse config data")
}

// TestValidateConnection_EndpointNotFound tests the scenario where the endpoint is not found in the ConfigData.
func TestValidateConnection_EndpointNotFound(t *testing.T) {
	// Arrange
	conn := &controllerapi.Connection{
		ConnectionId: "test-endpoint",
		ConfigData:   `{"other": "value"}`,
	}

	// Act
	err := validateConnection(conn)

	// Assert
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "endpoint not found in config data")
}

// TestValidateConnection_EndpointNotString tests the scenario where the endpoint is not a string.
func TestValidateConnection_EndpointNotString(t *testing.T) {
	// Arrange
	conn := &controllerapi.Connection{
		ConnectionId: "test-endpoint",
		ConfigData:   `{"endpoint": 123}`,
	}

	// Act
	err := validateConnection(conn)

	// Assert
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "endpoint is not a string")
}

// TestValidateConnection_EndpointEmpty tests the scenario where the endpoint is an empty string.
func TestValidateConnection_EndpointEmpty(t *testing.T) {
	// Arrange
	conn := &controllerapi.Connection{
		ConnectionId: "test-endpoint",
		ConfigData:   `{"endpoint": ""}`,
	}

	// Act
	err := validateConnection(conn)

	// Assert
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "endpoint cannot be empty")
}

// TestValidateConnection_EndpointMismatch tests the scenario
// where the endpoint in ConfigData does not match ConnectionId.
func TestValidateConnection_EndpointMismatch(t *testing.T) {
	// Arrange
	conn := &controllerapi.Connection{
		ConnectionId: "test-endpoint",
		ConfigData:   `{"endpoint": "different-endpoint"}`,
	}

	// Act
	err := validateConnection(conn)

	// Assert
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "endpoint in config data does not match connection ID")
}

// TestAddRoute_Success_WithDestNode tests the successful addition of a route with a destination node.
func TestAddRoute_Success_WithDestNode(t *testing.T) {
	// Arrange
	request := &controlplaneApi.AddRouteRequest{
		NodeId:     "source-node",
		DestNodeId: "dest-node",
		Subscription: &controllerapi.Subscription{
			Component_0: "org",
			Component_1: "namespace",
			Component_2: "agent",
			Id:          wrapperspb.UInt64(1),
		},
	}

	sourceNode := &controlplaneApi.NodeEntry{Id: "source-node"}
	destNode := &controlplaneApi.NodeEntry{Id: "dest-node"}
	routeID := "route-123"

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)

	mockNodeService.On("GetNodeByID", "source-node").Return(sourceNode, nil)
	mockNodeService.On("GetNodeByID", "dest-node").Return(destNode, nil)
	mockRouteService.On("AddRoute", mock.Anything, mock.AnythingOfType("routes.Route")).Return(routeID, nil)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		routeService: mockRouteService,
	}

	// Act
	result, err := s.AddRoute(context.Background(), request)

	// Assert
	assert.NoError(t, err)
	assert.True(t, result.Success)
	assert.Equal(t, routeID, result.RouteId)
	mockNodeService.AssertExpectations(t)
	mockRouteService.AssertExpectations(t)
}

// TestAddRoute_Success_WithConnection tests the successful addition of a route with a connection.
func TestAddRoute_Success_WithConnection(t *testing.T) {
	// Arrange
	request := &controlplaneApi.AddRouteRequest{
		NodeId: "source-node",
		Subscription: &controllerapi.Subscription{
			Component_0:  "org",
			Component_1:  "namespace",
			Component_2:  "agent",
			Id:           wrapperspb.UInt64(1),
			ConnectionId: "test-endpoint",
		},
		Connection: &controllerapi.Connection{
			ConnectionId: "test-endpoint",
			ConfigData:   `{"endpoint": "test-endpoint"}`,
		},
	}

	sourceNode := &controlplaneApi.NodeEntry{Id: "source-node"}
	routeID := "route-123"

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)

	mockNodeService.On("GetNodeByID", "source-node").Return(sourceNode, nil)
	mockRouteService.On("AddRoute", mock.Anything, mock.AnythingOfType("routes.Route")).Return(routeID, nil)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		routeService: mockRouteService,
	}

	// Act
	result, err := s.AddRoute(context.Background(), request)

	// Assert
	assert.NoError(t, err)
	assert.True(t, result.Success)
	assert.Equal(t, routeID, result.RouteId)
	mockNodeService.AssertExpectations(t)
	mockRouteService.AssertExpectations(t)
}

// TestAddRoute_InvalidSourceNode tests the scenario where the source node is invalid.
func TestAddRoute_InvalidSourceNode(t *testing.T) {
	// Arrange
	request := &controlplaneApi.AddRouteRequest{
		NodeId: "invalid-node",
		Subscription: &controllerapi.Subscription{
			Component_0: "org",
			Component_1: "namespace",
			Component_2: "agent",
			Id:          wrapperspb.UInt64(1),
		},
	}

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)

	mockNodeService.On("GetNodeByID", "invalid-node").Return(nil, errors.New("node not found"))

	s := &nbAPIService{
		nodeService:  mockNodeService,
		routeService: mockRouteService,
	}

	// Act
	result, err := s.AddRoute(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	assert.Contains(t, err.Error(), "invalid source nodeID")
	mockNodeService.AssertExpectations(t)
}

// TestAddRoute_MissingDestination tests the scenario where both destNodeId and connectionId are missing.
func TestAddRoute_MissingDestination(t *testing.T) {
	// Arrange
	request := &controlplaneApi.AddRouteRequest{
		NodeId: "source-node",
		Subscription: &controllerapi.Subscription{
			Component_0: "org",
			Component_1: "namespace",
			Component_2: "agent",
			Id:          wrapperspb.UInt64(1),
		},
	}

	sourceNode := &controlplaneApi.NodeEntry{Id: "source-node"}

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)

	mockNodeService.On("GetNodeByID", "source-node").Return(sourceNode, nil)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		routeService: mockRouteService,
	}

	// Act
	result, err := s.AddRoute(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	assert.Contains(t, err.Error(), "either destNodeId or connectionId must be provided")
	mockNodeService.AssertExpectations(t)
}

// TestDeleteRoute_Success_WithDestNode tests the successful deletion of a route with a destination node.
func TestDeleteRoute_Success_WithDestNode(t *testing.T) {
	// Arrange
	request := &controlplaneApi.DeleteRouteRequest{
		NodeId:     "source-node",
		DestNodeId: "dest-node",
		Subscription: &controllerapi.Subscription{
			Component_0: "org",
			Component_1: "namespace",
			Component_2: "agent",
			Id:          wrapperspb.UInt64(1),
		},
	}

	sourceNode := &controlplaneApi.NodeEntry{Id: "source-node"}
	destNode := &controlplaneApi.NodeEntry{Id: "dest-node"}

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)

	mockNodeService.On("GetNodeByID", "source-node").Return(sourceNode, nil)
	mockNodeService.On("GetNodeByID", "dest-node").Return(destNode, nil)
	mockRouteService.On("DeleteRoute", mock.Anything, mock.AnythingOfType("routes.Route")).Return(nil)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		routeService: mockRouteService,
	}

	// Act
	result, err := s.DeleteRoute(context.Background(), request)

	// Assert
	assert.NoError(t, err)
	assert.True(t, result.Success)
	mockNodeService.AssertExpectations(t)
	mockRouteService.AssertExpectations(t)
}

// TestDeleteRoute_Success_WithConnection tests the scenario
// where the route is deleted successfully with a connection ID.
func TestDeleteRoute_Success_WithConnection(t *testing.T) {
	// Arrange
	request := &controlplaneApi.DeleteRouteRequest{
		NodeId: "source-node",
		Subscription: &controllerapi.Subscription{
			Component_0:  "org",
			Component_1:  "namespace",
			Component_2:  "agent",
			Id:           wrapperspb.UInt64(1),
			ConnectionId: "test-endpoint",
		},
	}

	sourceNode := &controlplaneApi.NodeEntry{Id: "source-node"}

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)

	mockNodeService.On("GetNodeByID", "source-node").Return(sourceNode, nil)
	mockRouteService.On("DeleteRoute", mock.Anything, mock.AnythingOfType("routes.Route")).Return(nil)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		routeService: mockRouteService,
	}

	// Act
	result, err := s.DeleteRoute(context.Background(), request)

	// Assert
	assert.NoError(t, err)
	assert.True(t, result.Success)
	mockNodeService.AssertExpectations(t)
	mockRouteService.AssertExpectations(t)
}

// TestDeleteRoute_InvalidSourceNode tests the scenario where the source node is invalid.
func TestDeleteRoute_InvalidSourceNode(t *testing.T) {
	// Arrange
	request := &controlplaneApi.DeleteRouteRequest{
		NodeId: "invalid-node",
		Subscription: &controllerapi.Subscription{
			Component_0: "org",
			Component_1: "namespace",
			Component_2: "agent",
			Id:          wrapperspb.UInt64(1),
		},
	}

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)

	mockNodeService.On("GetNodeByID", "invalid-node").Return(nil, errors.New("node not found"))

	s := &nbAPIService{
		nodeService:  mockNodeService,
		routeService: mockRouteService,
	}

	// Act
	result, err := s.DeleteRoute(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	assert.Contains(t, err.Error(), "invalid source nodeID")
	mockNodeService.AssertExpectations(t)
}

// TestCreateChannel_Success tests the successful creation of a channel.
func TestCreateChannel_Success(t *testing.T) {
	// Arrange
	request := &controlplaneApi.CreateChannelRequest{
		Moderators: []string{"org/default/alice/0"},
	}
	expectedResponse := &controlplaneApi.CreateChannelResponse{}
	node := &controlplaneApi.NodeEntry{Id: "node1"}

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)
	mockGroupService := new(mockGroupService)

	nodeListResp := &controlplaneApi.NodeListResponse{
		Entries: []*controlplaneApi.NodeEntry{node},
	}
	subscriptionListResp := &controllerapi.SubscriptionListResponse{
		Entries: []*controllerapi.SubscriptionEntry{
			{
				Component_0: "org",
				Component_1: "default",
				Component_2: "alice",
			},
		},
	}

	mockNodeService.
		On("ListNodes", mock.Anything, mock.AnythingOfType("*controlplanev1.NodeListRequest")).
		Return(nodeListResp, nil)
	mockRouteService.On("ListSubscriptions", mock.Anything, node).Return(subscriptionListResp, nil)
	mockGroupService.On("CreateChannel", mock.Anything, request, node).Return(expectedResponse, nil)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		routeService: mockRouteService,
		groupService: mockGroupService,
	}

	// Act
	result, err := s.CreateChannel(context.Background(), request)

	// Assert
	assert.NoError(t, err)
	assert.Equal(t, expectedResponse, result)
	mockNodeService.AssertExpectations(t)
	mockRouteService.AssertExpectations(t)
	mockGroupService.AssertExpectations(t)
}

func TestCreateChannel_NoNodesAvailable(t *testing.T) {
	// Arrange
	request := &controlplaneApi.CreateChannelRequest{
		Moderators: []string{"org/default/alice/0"},
	}

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)
	mockGroupService := new(mockGroupService)

	nodeListResp := &controlplaneApi.NodeListResponse{
		Entries: []*controlplaneApi.NodeEntry{},
	}

	mockNodeService.
		On("ListNodes", mock.Anything, mock.AnythingOfType("*controlplanev1.NodeListRequest")).
		Return(nodeListResp, nil)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		routeService: mockRouteService,
		groupService: mockGroupService,
	}

	// Act
	result, err := s.CreateChannel(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	assert.Contains(t, err.Error(), "no nodes available")
	mockNodeService.AssertExpectations(t)
}

func TestCreateChannel_NodeServiceError(t *testing.T) {
	// Arrange
	request := &controlplaneApi.CreateChannelRequest{
		Moderators: []string{"org/default/alice/0"},
	}
	expectedError := errors.New("node service error")

	mockNodeService := new(mockNodeService)
	mockRouteService := new(mockRouteService)
	mockGroupService := new(mockGroupService)

	mockNodeService.
		On("ListNodes", mock.Anything, mock.AnythingOfType("*controlplanev1.NodeListRequest")).
		Return(nil, expectedError)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		routeService: mockRouteService,
		groupService: mockGroupService,
	}

	// Act
	result, err := s.CreateChannel(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	assert.Contains(t, err.Error(), "node service error")
	mockNodeService.AssertExpectations(t)
}

// TestDeleteChannel_Success tests the successful deletion of a channel.
func TestDeleteChannel_Success(t *testing.T) {
	// Arrange
	request := &controllerapi.DeleteChannelRequest{
		ChannelName: "test-channel",
	}
	expectedResponse := &controllerapi.Ack{
		Success: true,
	}

	channel := db.Channel{
		ID:         "test-channel",
		Moderators: []string{"org/default/alice/0"},
	}
	node1 := &controlplaneApi.NodeEntry{Id: "node1"}
	nodeListResp := &controlplaneApi.NodeListResponse{
		Entries: []*controlplaneApi.NodeEntry{node1},
	}

	mockGroupService := new(mockGroupService)

	mockGroupService.On("GetChannelDetails", mock.Anything, "test-channel").Return(channel, nil)

	mockNodeService := new(mockNodeService)
	mockNodeService.
		On("ListNodes", mock.Anything, mock.AnythingOfType("*controlplanev1.NodeListRequest")).
		Return(nodeListResp, nil)
	mockRouteService := new(mockRouteService)
	mockRouteService.
		On("ListSubscriptions", mock.Anything, node1).
		Return(&controllerapi.SubscriptionListResponse{Entries: []*controllerapi.SubscriptionEntry{}}, nil)

	mockGroupService.On("DeleteChannel", mock.Anything, request, node1).Return(expectedResponse, nil)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		routeService: mockRouteService,
		groupService: mockGroupService,
	}

	// Act
	result, err := s.DeleteChannel(context.Background(), request)

	// Assert
	assert.NoError(t, err)
	assert.Equal(t, expectedResponse, result)
	mockGroupService.AssertExpectations(t)
	mockNodeService.AssertExpectations(t)
	mockRouteService.AssertExpectations(t)
}

// TestDeleteChannel_ChannelNotFound tests the scenario where the channel to be deleted is not found.
func TestDeleteChannel_ChannelNotFound(t *testing.T) {
	// Arrange
	request := &controllerapi.DeleteChannelRequest{
		ChannelName: "nonexistent-channel",
	}
	expectedError := errors.New("channel not found")

	mockGroupService := new(mockGroupService)
	mockGroupService.On("GetChannelDetails", mock.Anything, "nonexistent-channel").Return(db.Channel{}, expectedError)

	s := &nbAPIService{
		groupService: mockGroupService,
	}
	// Act
	result, err := s.DeleteChannel(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	assert.Contains(t, err.Error(), "channel not found")
	mockGroupService.AssertExpectations(t)
}

// TestDeleteChannel_NoNodesAvailable tests the scenario where no nodes are available.
func TestDeleteChannel_NodeServiceError(t *testing.T) {
	// Arrange
	request := &controllerapi.DeleteChannelRequest{
		ChannelName: "test-channel",
	}
	expectedError := errors.New("node service error")

	channel := db.Channel{
		ID:         "test-channel",
		Moderators: []string{"org/default/alice/0"},
	}

	mockGroupService := new(mockGroupService)
	mockGroupService.On("GetChannelDetails", mock.Anything, "test-channel").Return(channel, nil)

	mockNodeService := new(mockNodeService)
	mockNodeService.
		On("ListNodes", mock.Anything, mock.AnythingOfType("*controlplanev1.NodeListRequest")).
		Return(nil, expectedError)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		groupService: mockGroupService,
	}

	// Act
	result, err := s.DeleteChannel(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	assert.Contains(t, err.Error(), "node service error")
	mockGroupService.AssertExpectations(t)
	mockNodeService.AssertExpectations(t)
}

// TestAddParticipant_Success tests the successful addition of a participant to a channel.
func TestAddParticipant_Success(t *testing.T) {
	// Arrange
	request := &controllerapi.AddParticipantRequest{
		ChannelName:     "test-channel",
		ParticipantName: "org/default/bob/0",
	}
	expectedResponse := &controllerapi.Ack{
		Success: true,
	}

	channel := db.Channel{
		ID:         "test-channel",
		Moderators: []string{"org/default/alice/0"},
	}
	node1 := &controlplaneApi.NodeEntry{Id: "node1"}
	nodeListResp := &controlplaneApi.NodeListResponse{
		Entries: []*controlplaneApi.NodeEntry{node1},
	}

	mockGroupService := new(mockGroupService)

	mockGroupService.On("GetChannelDetails", mock.Anything, "test-channel").Return(channel, nil)

	mockNodeService := new(mockNodeService)
	mockNodeService.
		On("ListNodes", mock.Anything, mock.AnythingOfType("*controlplanev1.NodeListRequest")).
		Return(nodeListResp, nil)
	mockRouteService := new(mockRouteService)
	mockRouteService.
		On("ListSubscriptions", mock.Anything, node1).
		Return(&controllerapi.SubscriptionListResponse{Entries: []*controllerapi.SubscriptionEntry{}}, nil)

	mockGroupService.On("AddParticipant", mock.Anything, request, node1).Return(expectedResponse, nil)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		routeService: mockRouteService,
		groupService: mockGroupService,
	}

	// Act
	result, err := s.AddParticipant(context.Background(), request)

	// Assert
	assert.NoError(t, err)
	assert.Equal(t, expectedResponse, result)
	mockGroupService.AssertExpectations(t)
	mockNodeService.AssertExpectations(t)
	mockRouteService.AssertExpectations(t)
}

// TestAddParticipant_ChannelNotFound tests the scenario
// where the channel to which a participant is to be added is not found.
func TestAddParticipant_ChannelNotFound(t *testing.T) {
	// Arrange
	request := &controllerapi.AddParticipantRequest{
		ChannelName:     "nonexistent-channel",
		ParticipantName: "org/default/bob/0",
	}
	expectedError := errors.New("channel not found")

	mockGroupService := new(mockGroupService)
	mockGroupService.On("GetChannelDetails", mock.Anything, "nonexistent-channel").Return(db.Channel{}, expectedError)

	s := &nbAPIService{
		groupService: mockGroupService,
	}
	// Act
	result, err := s.AddParticipant(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	assert.Contains(t, err.Error(), "channel not found")
	mockGroupService.AssertExpectations(t)
}

// TestAddParticipant_NodeServiceError tests the scenario where the NodeService returns an error.
func TestAddParticipant_NodeServiceError(t *testing.T) {
	// Arrange
	request := &controllerapi.AddParticipantRequest{
		ChannelName:     "test-channel",
		ParticipantName: "org/default/bob/0",
	}
	expectedError := errors.New("node service error")

	channel := db.Channel{
		ID:         "test-channel",
		Moderators: []string{"org/default/alice/0"},
	}

	mockGroupService := new(mockGroupService)
	mockGroupService.On("GetChannelDetails", mock.Anything, "test-channel").Return(channel, nil)

	mockNodeService := new(mockNodeService)
	mockNodeService.
		On("ListNodes", mock.Anything, mock.AnythingOfType("*controlplanev1.NodeListRequest")).
		Return(nil, expectedError)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		groupService: mockGroupService,
	}

	// Act
	result, err := s.AddParticipant(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	assert.Contains(t, err.Error(), "node service error")
	mockGroupService.AssertExpectations(t)
	mockNodeService.AssertExpectations(t)
}

// TestDeleteParticipant_Success tests the successful deletion of a participant from a channel.
func TestDeleteParticipant_Success(t *testing.T) {
	// Arrange
	request := &controllerapi.DeleteParticipantRequest{
		ChannelName:     "test-channel",
		ParticipantName: "org/default/bob/0",
	}
	expectedResponse := &controllerapi.Ack{
		Success: true,
	}

	channel := db.Channel{
		ID:         "test-channel",
		Moderators: []string{"org/default/alice/0"},
	}
	node1 := &controlplaneApi.NodeEntry{Id: "node1"}
	nodeListResp := &controlplaneApi.NodeListResponse{
		Entries: []*controlplaneApi.NodeEntry{node1},
	}

	mockGroupService := new(mockGroupService)

	mockGroupService.On("GetChannelDetails", mock.Anything, "test-channel").Return(channel, nil)

	mockNodeService := new(mockNodeService)
	mockNodeService.
		On("ListNodes", mock.Anything, mock.AnythingOfType("*controlplanev1.NodeListRequest")).
		Return(nodeListResp, nil)
	mockRouteService := new(mockRouteService)
	mockRouteService.
		On("ListSubscriptions", mock.Anything, node1).
		Return(&controllerapi.SubscriptionListResponse{Entries: []*controllerapi.SubscriptionEntry{}}, nil)

	mockGroupService.On("DeleteParticipant", mock.Anything, request, node1).Return(expectedResponse, nil)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		routeService: mockRouteService,
		groupService: mockGroupService,
	}

	// Act
	result, err := s.DeleteParticipant(context.Background(), request)

	// Assert
	assert.NoError(t, err)
	assert.Equal(t, expectedResponse, result)
	mockGroupService.AssertExpectations(t)
	mockNodeService.AssertExpectations(t)
	mockRouteService.AssertExpectations(t)
}

// TestDeleteParticipant_ChannelNotFound tests the scenario
// where the channel from which a participant is to be deleted is not found.
func TestDeleteParticipant_ChannelNotFound(t *testing.T) {
	// Arrange
	request := &controllerapi.DeleteParticipantRequest{
		ChannelName:     "nonexistent-channel",
		ParticipantName: "org/default/bob/0",
	}
	expectedError := errors.New("channel not found")

	mockGroupService := new(mockGroupService)
	mockGroupService.On("GetChannelDetails", mock.Anything, "nonexistent-channel").Return(db.Channel{}, expectedError)

	s := &nbAPIService{
		groupService: mockGroupService,
	}
	// Act
	result, err := s.DeleteParticipant(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	assert.Contains(t, err.Error(), "channel not found")
	mockGroupService.AssertExpectations(t)
}

// TestDeleteParticipant_NodeServiceError tests the scenario where the NodeService returns an error.
func TestDeleteParticipant_NodeServiceError(t *testing.T) {
	// Arrange
	request := &controllerapi.DeleteParticipantRequest{
		ChannelName:     "test-channel",
		ParticipantName: "org/default/bob/0",
	}
	expectedError := errors.New("node service error")

	channel := db.Channel{
		ID:         "test-channel",
		Moderators: []string{"org/default/alice/0"},
	}

	mockGroupService := new(mockGroupService)
	mockGroupService.On("GetChannelDetails", mock.Anything, "test-channel").Return(channel, nil)

	mockNodeService := new(mockNodeService)
	mockNodeService.
		On("ListNodes", mock.Anything, mock.AnythingOfType("*controlplanev1.NodeListRequest")).
		Return(nil, expectedError)

	s := &nbAPIService{
		nodeService:  mockNodeService,
		groupService: mockGroupService,
	}

	// Act
	result, err := s.DeleteParticipant(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	assert.Contains(t, err.Error(), "node service error")
	mockGroupService.AssertExpectations(t)
	mockNodeService.AssertExpectations(t)
}

// TestListChannels_Success tests the successful retrieval of channels.
func TestListChannels_Success(t *testing.T) {
	// Arrange
	request := &controllerapi.ListChannelsRequest{}
	expectedResponse := &controllerapi.ListChannelsResponse{
		ChannelName: []string{"channel1", "channel2"},
	}

	mockGroupService := new(mockGroupService)
	mockGroupService.On("ListChannels", mock.Anything, request).Return(expectedResponse, nil)

	s := &nbAPIService{
		groupService: mockGroupService,
	}

	// Act
	result, err := s.ListChannels(context.Background(), request)

	// Assert
	assert.NoError(t, err)
	assert.Equal(t, expectedResponse, result)
	mockGroupService.AssertExpectations(t)
}

// TestListChannels_Error tests the scenario where the GroupService returns an error.
func TestListChannels_Error(t *testing.T) {
	// Arrange
	request := &controllerapi.ListChannelsRequest{}
	expectedError := errors.New("service error")

	mockGroupService := new(mockGroupService)
	mockGroupService.On("ListChannels", mock.Anything, request).Return(nil, expectedError)

	s := &nbAPIService{
		groupService: mockGroupService,
	}

	// Act
	result, err := s.ListChannels(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	assert.Equal(t, expectedError, err)
	mockGroupService.AssertExpectations(t)
}

// TestListParticipants_Success tests the successful retrieval of participants.
func TestListParticipants_Success(t *testing.T) {
	// Arrange
	request := &controllerapi.ListParticipantsRequest{}
	expectedResponse := &controllerapi.ListParticipantsResponse{
		ParticipantName: []string{"participant1", "participant2"},
	}

	mockGroupService := new(mockGroupService)
	mockGroupService.On("ListParticipants", mock.Anything, request).Return(expectedResponse, nil)

	s := &nbAPIService{
		groupService: mockGroupService,
	}

	// Act
	result, err := s.ListParticipants(context.Background(), request)

	// Assert
	assert.NoError(t, err)
	assert.Equal(t, expectedResponse, result)
	mockGroupService.AssertExpectations(t)
}

// TestListParticipants_Error tests the scenario where the GroupService returns an error.
func TestListParticipants_Error(t *testing.T) {
	// Arrange
	request := &controllerapi.ListParticipantsRequest{}
	expectedError := errors.New("service error")

	mockGroupService := new(mockGroupService)
	mockGroupService.On("ListParticipants", mock.Anything, request).Return(nil, expectedError)

	s := &nbAPIService{
		groupService: mockGroupService,
	}

	// Act
	result, err := s.ListParticipants(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	assert.Equal(t, expectedError, err)
	mockGroupService.AssertExpectations(t)
}

// TestIsSubscriptionSameAsModerator_Match tests the scenario where the subscription matches the moderator details.
func TestIsSubscriptionSameAsModerator_Match(t *testing.T) {
	// Arrange
	subscription := &controllerapi.SubscriptionEntry{
		Component_0: "org",
		Component_1: "namespace",
		Component_2: "agent",
	}

	// Act
	result := isSubscriptionSameAsModerator(subscription, "org", "namespace", "agent")

	// Assert
	assert.True(t, result)
}

// TestIsSubscriptionSameAsModerator_NoMatch tests the scenario
// where the subscription does not match the moderator details.
func TestIsSubscriptionSameAsModerator_NoMatch(t *testing.T) {
	// Arrange
	subscription := &controllerapi.SubscriptionEntry{
		Component_0: "org",
		Component_1: "namespace",
		Component_2: "agent",
	}

	// Act
	result := isSubscriptionSameAsModerator(subscription, "different", "namespace", "agent")

	// Assert
	assert.False(t, result)
}
