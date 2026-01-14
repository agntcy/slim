package nbapiservice

import (
	"context"
	"errors"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/mock"

	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
)

func TestNodeService_GetNodeByID_Success(t *testing.T) {
	// Arrange
	nodeID := "test-node-1"
	endPoint := "http://test-endpoint:8080"
	externalEndpoint := "http://external-endpoint:8080"
	expectedNode := &db.Node{
		ID: nodeID,
		ConnDetails: []db.ConnectionDetails{
			{
				Endpoint:         endPoint,
				MTLSRequired:     true,
				ExternalEndpoint: &externalEndpoint,
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
	// Check metadata for external_endpoint
	assert.NotNil(t, result.Connections[0].Metadata)
	assert.NotNil(t, result.Connections[0].Metadata.Fields)
	assert.Equal(t, externalEndpoint,
		result.Connections[0].Metadata.Fields["external_endpoint"].GetStringValue())

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
	externalEndpoint2 := "http://external2:8080"
	expectedNode := &db.Node{
		ID: nodeID,
		ConnDetails: []db.ConnectionDetails{
			{
				Endpoint:         endPoint1,
				MTLSRequired:     true,
				ExternalEndpoint: &externalEndpoint1,
			},
			{
				Endpoint:         endPoint2,
				MTLSRequired:     false,
				ExternalEndpoint: &externalEndpoint2,
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
	assert.NotNil(t, result.Connections[0].Metadata)
	assert.NotNil(t, result.Connections[0].Metadata.Fields)
	assert.Equal(t, externalEndpoint1,
		result.Connections[0].Metadata.Fields["external_endpoint"].GetStringValue())

	// Check second connection
	assert.Equal(t, endPoint2, result.Connections[1].Endpoint)
	assert.False(t, result.Connections[1].MtlsRequired)
	assert.NotNil(t, result.Connections[1].Metadata)
	assert.NotNil(t, result.Connections[1].Metadata.Fields)
	assert.Equal(t, externalEndpoint2,
		result.Connections[1].Metadata.Fields["external_endpoint"].GetStringValue())

	mockDB.AssertExpectations(t)
}

func TestNodeService_ListNodes_Success(t *testing.T) {
	// Arrange
	nodeID1 := "test-node-1"
	nodeID2 := "test-node-2"
	endPoint1 := "http://endpoint1:8080"
	externalEndpoint1 := "http://external1:8080"

	storedNodes := []db.Node{
		{
			ID: nodeID1,
			ConnDetails: []db.ConnectionDetails{
				{
					Endpoint:         endPoint1,
					MTLSRequired:     true,
					ExternalEndpoint: &externalEndpoint1,
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
	assert.NotNil(t, result.Entries[0].Connections[0].Metadata)
	assert.NotNil(t, result.Entries[0].Connections[0].Metadata.Fields)
	assert.Equal(t, externalEndpoint1, result.Entries[0].
		Connections[0].Metadata.Fields["external_endpoint"].GetStringValue())
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
