package nbapiservice

import (
	"context"
	"errors"
	"reflect"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/mock"

	controllerv1 "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
)

type mockDataAccess struct {
	mock.Mock
}

// AddRoute implements db.DataAccess.
func (m *mockDataAccess) AddRoute(route db.Route) string {
	panic("unimplemented")
}

// DeleteChannel implements db.DataAccess.
func (m *mockDataAccess) DeleteChannel(channelID string) error {
	panic("unimplemented")
}

// DeleteNode implements db.DataAccess.
func (m *mockDataAccess) DeleteNode(id string) error {
	panic("unimplemented")
}

// DeleteRoute implements db.DataAccess.
func (m *mockDataAccess) DeleteRoute(routeID string) error {
	panic("unimplemented")
}

// GetChannel implements db.DataAccess.
func (m *mockDataAccess) GetChannel(channelID string) (db.Channel, error) {
	panic("unimplemented")
}

// GetRouteByID implements db.DataAccess.
func (m *mockDataAccess) GetRouteByID(routeID string) *db.Route {
	panic("unimplemented")
}

// GetRoutesForNodeID implements db.DataAccess.
func (m *mockDataAccess) GetRoutesForNodeID(nodeID string) []db.Route {
	panic("unimplemented")
}

// ListChannels implements db.DataAccess.
func (m *mockDataAccess) ListChannels() ([]db.Channel, error) {
	panic("unimplemented")
}

// MarkRouteAsDeleted implements db.DataAccess.
func (m *mockDataAccess) MarkRouteAsDeleted(routeID string) error {
	panic("unimplemented")
}

// SaveChannel implements db.DataAccess.
func (m *mockDataAccess) SaveChannel(channelID string, moderators []string) error {
	panic("unimplemented")
}

// SaveNode implements db.DataAccess.
func (m *mockDataAccess) SaveNode(node db.Node) (string, error) {
	panic("unimplemented")
}

// UpdateChannel implements db.DataAccess.
func (m *mockDataAccess) UpdateChannel(channel db.Channel) error {
	panic("unimplemented")
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
func (m *mockNodeCommandHandler) AddStream(ctx context.Context, nodeID string, stream controllerv1.ControllerService_OpenControlChannelServer) {
	panic("unimplemented")
}

// GetConnectionStatus implements nodecontrol.NodeCommandHandler.
func (m *mockNodeCommandHandler) GetConnectionStatus(ctx context.Context, nodeID string) (nodecontrol.NodeStatus, error) {
	args := m.Called(ctx, nodeID)
	return args.Get(0).(nodecontrol.NodeStatus), args.Error(1)
}

// RemoveStream implements nodecontrol.NodeCommandHandler.
func (m *mockNodeCommandHandler) RemoveStream(ctx context.Context, nodeID string) error {
	panic("unimplemented")
}

// ResponseReceived implements nodecontrol.NodeCommandHandler.
func (m *mockNodeCommandHandler) ResponseReceived(ctx context.Context, nodeID string, command *controllerv1.ControlMessage) {
	panic("unimplemented")
}

// SendMessage implements nodecontrol.NodeCommandHandler.
func (m *mockNodeCommandHandler) SendMessage(ctx context.Context, nodeID string, configurationCommand *controllerv1.ControlMessage) error {
	panic("unimplemented")
}

// UpdateConnectionStatus implements nodecontrol.NodeCommandHandler.
func (m *mockNodeCommandHandler) UpdateConnectionStatus(nctx context.Context, odeID string, status nodecontrol.NodeStatus) {
	panic("unimplemented")
}

// WaitForResponse implements nodecontrol.NodeCommandHandler.
func (m *mockNodeCommandHandler) WaitForResponse(ctx context.Context, nodeID string, messageType reflect.Type, messageID string) (*controllerv1.ControlMessage, error) {
	panic("unimplemented")
}

func TestNodeService_GetNodeByID_Success(t *testing.T) {
	// Arrange
	nodeID := "test-node-1"
	endPoint := "http://test-endpoint:8080"
	externalEndpoint := "http://external-endpoint:8080"
	groupName := "test-group"
	expectedNode := &db.Node{
		ID: nodeID,
		ConnDetails: []db.ConnectionDetails{
			{
				Endpoint:         endPoint,
				MTLSRequired:     true,
				ExternalEndpoint: &externalEndpoint,
				GroupName:        &groupName,
			},
		},
	}

	mockDB := new(mockDataAccess)
	mockCmdHandler := new(mockNodeCommandHandler)
	mockDB.On("GetNode", nodeID).Return(expectedNode, nil)

	service := NewNodeService(mockDB, mockCmdHandler)

	// Act
	result, err := service.GetNodeByID(nodeID)

	// Assert
	assert.NoError(t, err)
	assert.NotNil(t, result)
	assert.Equal(t, nodeID, result.Id)
	assert.Len(t, result.Connections, 1)
	assert.Equal(t, endPoint, result.Connections[0].Endpoint)
	assert.True(t, result.Connections[0].MtlsRequired)
	assert.Equal(t, &externalEndpoint, result.Connections[0].ExternalEndpoint)
	assert.Equal(t, &groupName, result.Connections[0].GroupName)

	mockDB.AssertExpectations(t)
}

func TestNodeService_GetNodeByID_NodeNotFound(t *testing.T) {
	// Arrange
	nodeID := "non-existent-node"
	expectedError := errors.New("node not found")

	mockDB := new(mockDataAccess)
	mockCmdHandler := new(mockNodeCommandHandler)
	mockDB.On("GetNode", nodeID).Return(nil, expectedError)

	service := NewNodeService(mockDB, mockCmdHandler)

	// Act
	result, err := service.GetNodeByID(nodeID)

	// Assert
	assert.Error(t, err)
	assert.Nil(t, result)
	assert.Equal(t, expectedError, err)

	mockDB.AssertExpectations(t)
}

func TestNodeService_GetNodeByID_EmptyConnections(t *testing.T) {
	// Arrange
	nodeID := "test-node-2"
	expectedNode := &db.Node{
		ID:          nodeID,
		ConnDetails: []db.ConnectionDetails{},
	}

	mockDB := new(mockDataAccess)
	mockCmdHandler := new(mockNodeCommandHandler)
	mockDB.On("GetNode", nodeID).Return(expectedNode, nil)

	service := NewNodeService(mockDB, mockCmdHandler)

	// Act
	result, err := service.GetNodeByID(nodeID)

	// Assert
	assert.NoError(t, err)
	assert.NotNil(t, result)
	assert.Equal(t, nodeID, result.Id)
	assert.Empty(t, result.Connections)

	mockDB.AssertExpectations(t)
}

func TestNodeService_GetNodeByID_MultipleConnections(t *testing.T) {
	// Arrange
	nodeID := "test-node-3"
	endPoint1 := "http://endpoint1:8080"
	endPoint2 := "http://endpoint2:8080"
	externalEndpoint1 := "http://external1:8080"
	groupName1 := "group1"
	externalEndpoint2 := "http://external2:8080"
	groupName2 := "group2"
	expectedNode := &db.Node{
		ID: nodeID,
		ConnDetails: []db.ConnectionDetails{
			{
				Endpoint:         endPoint1,
				MTLSRequired:     true,
				ExternalEndpoint: &externalEndpoint1,
				GroupName:        &groupName1,
			},
			{
				Endpoint:         endPoint2,
				MTLSRequired:     false,
				ExternalEndpoint: &externalEndpoint2,
				GroupName:        &groupName2,
			},
		},
	}

	mockDB := new(mockDataAccess)
	mockCmdHandler := new(mockNodeCommandHandler)
	mockDB.On("GetNode", nodeID).Return(expectedNode, nil)

	service := NewNodeService(mockDB, mockCmdHandler)

	// Act
	result, err := service.GetNodeByID(nodeID)

	// Assert
	assert.NoError(t, err)
	assert.NotNil(t, result)
	assert.Equal(t, nodeID, result.Id)
	assert.Len(t, result.Connections, 2)

	// Check first connection
	assert.Equal(t, endPoint1, result.Connections[0].Endpoint)
	assert.True(t, result.Connections[0].MtlsRequired)
	assert.Equal(t, &externalEndpoint1, result.Connections[0].ExternalEndpoint)
	assert.Equal(t, &groupName1, result.Connections[0].GroupName)

	// Check second connection
	assert.Equal(t, endPoint2, result.Connections[1].Endpoint)
	assert.False(t, result.Connections[1].MtlsRequired)
	assert.Equal(t, &externalEndpoint2, result.Connections[1].ExternalEndpoint)
	assert.Equal(t, &groupName2, result.Connections[1].GroupName)

	mockDB.AssertExpectations(t)
}

func TestNodeService_ListNodes_Success(t *testing.T) {
	// Arrange
	nodeID1 := "test-node-1"
	nodeID2 := "test-node-2"
	endPoint1 := "http://endpoint1:8080"
	externalEndpoint1 := "http://external1:8080"
	groupName1 := "group1"

	storedNodes := []db.Node{
		{
			ID: nodeID1,
			ConnDetails: []db.ConnectionDetails{
				{
					Endpoint:         endPoint1,
					MTLSRequired:     true,
					ExternalEndpoint: &externalEndpoint1,
					GroupName:        &groupName1,
				},
			},
		},
		{
			ID:          nodeID2,
			ConnDetails: []db.ConnectionDetails{},
		},
	}

	mockDB := new(mockDataAccess)
	mockCmdHandler := new(mockNodeCommandHandler)
	mockDB.On("ListNodes").Return(storedNodes)
	mockCmdHandler.On("GetConnectionStatus", mock.Anything, nodeID1).Return(nodecontrol.NodeStatusConnected, nil)
	mockCmdHandler.On("GetConnectionStatus", mock.Anything, nodeID2).Return(nodecontrol.NodeStatusNotConnected, nil)

	service := NewNodeService(mockDB, mockCmdHandler)
	req := &controlplaneApi.NodeListRequest{}

	// Act
	result, err := service.ListNodes(context.Background(), req)

	// Assert
	assert.NoError(t, err)
	assert.NotNil(t, result)
	assert.Len(t, result.Entries, 2)

	// Check first node
	assert.Equal(t, nodeID1, result.Entries[0].Id)
	assert.Len(t, result.Entries[0].Connections, 1)
	assert.Equal(t, endPoint1, result.Entries[0].Connections[0].Endpoint)
	assert.True(t, result.Entries[0].Connections[0].MtlsRequired)
	assert.Equal(t, &externalEndpoint1, result.Entries[0].Connections[0].ExternalEndpoint)
	assert.Equal(t, &groupName1, result.Entries[0].Connections[0].GroupName)
	assert.Equal(t, controlplaneApi.NodeStatus_CONNECTED, result.Entries[0].Status)

	// Check second node
	assert.Equal(t, nodeID2, result.Entries[1].Id)
	assert.Empty(t, result.Entries[1].Connections)
	assert.Equal(t, controlplaneApi.NodeStatus_NOT_CONNECTED, result.Entries[1].Status)

	mockDB.AssertExpectations(t)
	mockCmdHandler.AssertExpectations(t)
}

func TestNodeService_ListNodes_EmptyList(t *testing.T) {
	// Arrange
	storedNodes := []db.Node{}

	mockDB := new(mockDataAccess)
	mockCmdHandler := new(mockNodeCommandHandler)
	mockDB.On("ListNodes").Return(storedNodes)

	service := NewNodeService(mockDB, mockCmdHandler)
	req := &controlplaneApi.NodeListRequest{}

	// Act
	result, err := service.ListNodes(context.Background(), req)

	// Assert
	assert.NoError(t, err)
	assert.NotNil(t, result)
	assert.Empty(t, result.Entries)

	mockDB.AssertExpectations(t)
}

func TestNodeService_ListNodes_StatusError(t *testing.T) {
	// Arrange
	nodeID := "test-node-1"
	storedNodes := []db.Node{
		{
			ID:          nodeID,
			ConnDetails: []db.ConnectionDetails{},
		},
	}
	statusError := errors.New("connection status error")

	mockDB := new(mockDataAccess)
	mockCmdHandler := new(mockNodeCommandHandler)
	mockDB.On("ListNodes").Return(storedNodes)
	mockCmdHandler.On("GetConnectionStatus", mock.Anything, nodeID).Return(nodecontrol.NodeStatusUnknown, statusError)

	service := NewNodeService(mockDB, mockCmdHandler)
	req := &controlplaneApi.NodeListRequest{}

	// Act
	result, err := service.ListNodes(context.Background(), req)

	// Assert
	assert.NoError(t, err)
	assert.NotNil(t, result)
	assert.Len(t, result.Entries, 1)
	assert.Equal(t, nodeID, result.Entries[0].Id)
	// Status should be default (0) when error occurs
	assert.Equal(t, controlplaneApi.NodeStatus(0), result.Entries[0].Status)

	mockDB.AssertExpectations(t)
	mockCmdHandler.AssertExpectations(t)
}

func TestNodeService_ListNodes_MultipleNodesWithDifferentStatuses(t *testing.T) {
	// Arrange
	nodeID1 := "connected-node"
	nodeID2 := "disconnected-node"
	nodeID3 := "unknown-status-node"

	storedNodes := []db.Node{
		{ID: nodeID1, ConnDetails: []db.ConnectionDetails{}},
		{ID: nodeID2, ConnDetails: []db.ConnectionDetails{}},
		{ID: nodeID3, ConnDetails: []db.ConnectionDetails{}},
	}

	mockDB := new(mockDataAccess)
	mockCmdHandler := new(mockNodeCommandHandler)
	mockDB.On("ListNodes").Return(storedNodes)
	mockCmdHandler.On("GetConnectionStatus", mock.Anything, nodeID1).Return(nodecontrol.NodeStatusConnected, nil)
	mockCmdHandler.On("GetConnectionStatus", mock.Anything, nodeID2).Return(nodecontrol.NodeStatusNotConnected, nil)
	mockCmdHandler.On("GetConnectionStatus", mock.Anything, nodeID3).Return(nodecontrol.NodeStatusUnknown, nil)

	service := NewNodeService(mockDB, mockCmdHandler)
	req := &controlplaneApi.NodeListRequest{}

	// Act
	result, err := service.ListNodes(context.Background(), req)

	// Assert
	assert.NoError(t, err)
	assert.NotNil(t, result)
	assert.Len(t, result.Entries, 3)

	assert.Equal(t, nodeID1, result.Entries[0].Id)
	assert.Equal(t, controlplaneApi.NodeStatus_CONNECTED, result.Entries[0].Status)

	assert.Equal(t, nodeID2, result.Entries[1].Id)
	assert.Equal(t, controlplaneApi.NodeStatus_NOT_CONNECTED, result.Entries[1].Status)

	assert.Equal(t, nodeID3, result.Entries[2].Id)
	assert.Equal(t, controlplaneApi.NodeStatus(0), result.Entries[2].Status) // Default status for unknown

	mockDB.AssertExpectations(t)
	mockCmdHandler.AssertExpectations(t)
}
