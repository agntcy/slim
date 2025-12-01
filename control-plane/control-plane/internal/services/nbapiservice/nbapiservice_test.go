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
)

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

	mockGroupService := new(mockGroupService)
	mockGroupService.On("CreateChannel", mock.Anything, request).Return(expectedResponse, nil)

	s := &nbAPIService{
		groupService: mockGroupService,
	}

	// Act
	result, err := s.CreateChannel(context.Background(), request)

	// Assert
	assert.NoError(t, err)
	assert.Equal(t, expectedResponse, result)
	mockGroupService.AssertExpectations(t)
}

func TestCreateChannel_GroupServiceError(t *testing.T) {
	// Arrange
	request := &controlplaneApi.CreateChannelRequest{
		Moderators: []string{"org/default/alice/0"},
	}
	expectedError := errors.New("group service error")

	mockGroupService := new(mockGroupService)
	mockGroupService.On("CreateChannel", mock.Anything, request).Return(nil, expectedError)

	s := &nbAPIService{
		groupService: mockGroupService,
	}

	// Act
	result, err := s.CreateChannel(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	mockGroupService.AssertExpectations(t)
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

	mockGroupService := new(mockGroupService)
	mockGroupService.On("DeleteChannel", mock.Anything, request).Return(expectedResponse, nil)

	s := &nbAPIService{
		groupService: mockGroupService,
	}

	// Act
	result, err := s.DeleteChannel(context.Background(), request)

	// Assert
	assert.NoError(t, err)
	assert.Equal(t, expectedResponse, result)
	mockGroupService.AssertExpectations(t)
}

// TestDeleteChannel_GroupServiceError tests the scenario where the group service returns an error.
func TestDeleteChannel_GroupServiceError(t *testing.T) {
	// Arrange
	request := &controllerapi.DeleteChannelRequest{
		ChannelName: "test-channel",
	}
	expectedError := errors.New("group service error")

	mockGroupService := new(mockGroupService)
	mockGroupService.On("DeleteChannel", mock.Anything, request).Return(nil, expectedError)

	s := &nbAPIService{
		groupService: mockGroupService,
	}

	// Act
	result, err := s.DeleteChannel(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	mockGroupService.AssertExpectations(t)
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

	mockGroupService := new(mockGroupService)
	mockGroupService.On("AddParticipant", mock.Anything, request).Return(expectedResponse, nil)

	s := &nbAPIService{
		groupService: mockGroupService,
	}

	// Act
	result, err := s.AddParticipant(context.Background(), request)

	// Assert
	assert.NoError(t, err)
	assert.Equal(t, expectedResponse, result)
	mockGroupService.AssertExpectations(t)
}

// TestAddParticipant_GroupServiceError tests the scenario where the group service returns an error.
func TestAddParticipant_GroupServiceError(t *testing.T) {
	// Arrange
	request := &controllerapi.AddParticipantRequest{
		ChannelName:     "test-channel",
		ParticipantName: "org/default/bob/0",
	}
	expectedError := errors.New("group service error")

	mockGroupService := new(mockGroupService)
	mockGroupService.On("AddParticipant", mock.Anything, request).Return(nil, expectedError)

	s := &nbAPIService{
		groupService: mockGroupService,
	}

	// Act
	result, err := s.AddParticipant(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	mockGroupService.AssertExpectations(t)
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

	mockGroupService := new(mockGroupService)
	mockGroupService.On("DeleteParticipant", mock.Anything, request).Return(expectedResponse, nil)

	s := &nbAPIService{
		groupService: mockGroupService,
	}

	// Act
	result, err := s.DeleteParticipant(context.Background(), request)

	// Assert
	assert.NoError(t, err)
	assert.Equal(t, expectedResponse, result)
	mockGroupService.AssertExpectations(t)
}

// TestDeleteParticipant_GroupServiceError tests the scenario where the group service returns an error.
func TestDeleteParticipant_GroupServiceError(t *testing.T) {
	// Arrange
	request := &controllerapi.DeleteParticipantRequest{
		ChannelName:     "test-channel",
		ParticipantName: "org/default/bob/0",
	}
	expectedError := errors.New("group service error")

	mockGroupService := new(mockGroupService)
	mockGroupService.On("DeleteParticipant", mock.Anything, request).Return(nil, expectedError)

	s := &nbAPIService{
		groupService: mockGroupService,
	}

	// Act
	result, err := s.DeleteParticipant(context.Background(), request)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	mockGroupService.AssertExpectations(t)
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
