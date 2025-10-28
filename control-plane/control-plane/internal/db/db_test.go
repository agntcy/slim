// dataaccess_test.go - comprehensive tests for any DataAccess implementation
package db

import (
	"fmt"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"google.golang.org/protobuf/types/known/wrapperspb"
)

// DataAccessImplementation represents a factory function for creating DataAccess instances
type DataAccessImplementation struct {
	Name    string
	Factory func() DataAccess
	Cleanup func(DataAccess) // Optional cleanup function
}

// getDataAccessImplementations returns all DataAccess implementations to test
func getDataAccessImplementations() []DataAccessImplementation {
	return []DataAccessImplementation{
		{
			Name: "InMemoryDBService",
			Factory: func() DataAccess {
				return NewInMemoryDBService()
			},
			Cleanup: func(da DataAccess) {
				// InMemory doesn't need cleanup
			},
		},
		// Add other implementations here as they become available
		// {
		//     Name: "PostgreSQLDBService",
		//     Factory: func() DataAccess {
		//         return NewPostgreSQLDBService(testConnectionString)
		//     },
		//     Cleanup: func(da DataAccess) {
		//         // Clean up database connections, test data, etc.
		//     },
		// },
	}
}

// TestDataAccess_RouteOperations tests all route-related operations
func TestDataAccess_RouteOperations(t *testing.T) {
	implementations := getDataAccessImplementations()

	for _, impl := range implementations {
		t.Run(impl.Name, func(t *testing.T) {
			da := impl.Factory()
			defer impl.Cleanup(da)

			testRouteOperations(t, da)
		})
	}
}

func testRouteOperations(t *testing.T, da DataAccess) {
	// Test AddRoute
	route1 := Route{
		SourceNodeID: "node1",
		DestNodeID:   "node2",
		Component0:   "org",
		Component1:   "service",
		Component2:   "method",
		ComponentID:  wrapperspb.UInt64(123),
		Status:       RouteStatusFailed,
		LastUpdated:  time.Now(),
	}

	routeID1 := da.AddRoute(route1)
	require.NotEmpty(t, routeID1, "AddRoute should return non-empty route ID")

	route2 := Route{
		SourceNodeID: "node1",
		DestNodeID:   "node3",
		Component0:   "org",
		Component1:   "service2",
		Component2:   "method2",
		ComponentID:  wrapperspb.UInt64(456),
		Status:       RouteStatusFailed,
		LastUpdated:  time.Now(),
	}

	routeID2 := da.AddRoute(route2)
	require.NotEmpty(t, routeID2, "AddRoute should return non-empty route ID")
	require.NotEqual(t, routeID1, routeID2, "Route IDs should be unique")

	// Test GetRouteByID
	retrievedRoute := da.GetRouteByID(routeID1)
	require.NotNil(t, retrievedRoute, "GetRouteByID should return the route")
	assert.Equal(t, route1.SourceNodeID, retrievedRoute.SourceNodeID)
	assert.Equal(t, route1.DestNodeID, retrievedRoute.DestNodeID)
	assert.Equal(t, route1.Component0, retrievedRoute.Component0)
	assert.Equal(t, route1.Component1, retrievedRoute.Component1)
	assert.Equal(t, route1.Component2, retrievedRoute.Component2)
	assert.Equal(t, route1.ComponentID.GetValue(), retrievedRoute.ComponentID.GetValue())

	// Test GetRouteByID with non-existent ID
	nonExistentRoute := da.GetRouteByID("non-existent-id")
	assert.Nil(t, nonExistentRoute, "GetRouteByID should return nil for non-existent route")

	// Test GetRoutesForNodeID
	node1Routes := da.GetRoutesForNodeID("node1")
	assert.Len(t, node1Routes, 2, "node1 should have 2 routes")

	node2Routes := da.GetRoutesForNodeID("node2")
	assert.Len(t, node2Routes, 0, "node2 should have 0 routes as source")

	nonExistentNodeRoutes := da.GetRoutesForNodeID("non-existent-node")
	assert.Len(t, nonExistentNodeRoutes, 0, "non-existent node should have 0 routes")

	// Test FilterRoutesBySourceAndDestination
	filteredRoutes := da.FilterRoutesBySourceAndDestination("node1", "node2")
	assert.Len(t, filteredRoutes, 1, "Should find 1 route from node1 to node2")
	assert.Equal(t, routeID1, filteredRoutes[0].GetID())

	filteredRoutes = da.FilterRoutesBySourceAndDestination("node1", "node3")
	assert.Len(t, filteredRoutes, 1, "Should find 1 route from node1 to node3")

	filteredRoutes = da.FilterRoutesBySourceAndDestination("node2", "node3")
	assert.Len(t, filteredRoutes, 0, "Should find 0 routes from node2 to node3")

	// Test MarkRouteAsApplied
	err := da.MarkRouteAsApplied(routeID1)
	assert.NoError(t, err, "MarkRouteAsApplied should not return error")

	updatedRoute := da.GetRouteByID(routeID1)
	require.NotNil(t, updatedRoute)
	assert.Equal(t, RouteStatusApplied, updatedRoute.Status, "Route status should be Applied")

	// Test MarkRouteAsApplied with non-existent ID
	err = da.MarkRouteAsApplied("non-existent-id")
	assert.Error(t, err, "MarkRouteAsApplied should return error for non-existent route")

	// Test MarkRouteAsFailed
	failureMsg := "Connection timeout"
	err = da.MarkRouteAsFailed(routeID2, failureMsg)
	assert.NoError(t, err, "MarkRouteAsFailed should not return error")

	failedRoute := da.GetRouteByID(routeID2)
	require.NotNil(t, failedRoute)
	assert.Equal(t, RouteStatusFailed, failedRoute.Status, "Route status should be Failed")
	assert.Equal(t, failureMsg, failedRoute.StatusMsg, "Status message should be set")

	// Test MarkRouteAsFailed with non-existent ID
	err = da.MarkRouteAsFailed("non-existent-id", "some error")
	assert.Error(t, err, "MarkRouteAsFailed should return error for non-existent route")

	// Test MarkRouteAsDeleted
	err = da.MarkRouteAsDeleted(routeID1)
	assert.NoError(t, err, "MarkRouteAsDeleted should not return error")

	deletedRoute := da.GetRouteByID(routeID1)
	require.NotNil(t, deletedRoute)
	assert.True(t, deletedRoute.Deleted, "Route should be marked as deleted")

	// Test MarkRouteAsDeleted with non-existent ID
	err = da.MarkRouteAsDeleted("non-existent-id")
	assert.Error(t, err, "MarkRouteAsDeleted should return error for non-existent route")

	// Test DeleteRoute
	err = da.DeleteRoute(routeID2)
	assert.NoError(t, err, "DeleteRoute should not return error")

	deletedRoute = da.GetRouteByID(routeID2)
	assert.Nil(t, deletedRoute, "Route should be completely removed")

	// Test DeleteRoute with non-existent ID
	err = da.DeleteRoute("non-existent-id")
	assert.Error(t, err, "DeleteRoute should return error for non-existent route")

	// Test ComponentID nil handling
	routeWithNilComponentID := Route{
		SourceNodeID: "node4",
		DestNodeID:   "node5",
		Component0:   "org",
		Component1:   "service",
		Component2:   "method",
		ComponentID:  nil, // Test nil ComponentID
		Status:       RouteStatusFailed,
		LastUpdated:  time.Now(),
	}

	routeID3 := da.AddRoute(routeWithNilComponentID)
	require.NotEmpty(t, routeID3, "AddRoute should handle nil ComponentID")

	retrievedNilRoute := da.GetRouteByID(routeID3)
	require.NotNil(t, retrievedNilRoute)
	assert.Nil(t, retrievedNilRoute.ComponentID, "ComponentID should remain nil")

	// Test route with DestEndpoint instead of DestNodeID
	routeWithEndpoint := Route{
		SourceNodeID:   "node6",
		DestEndpoint:   "external.service.com:8080",
		ConnConfigData: `{"endpoint": "external.service.com:8080", "tls": true}`,
		Component0:     "org",
		Component1:     "external",
		Component2:     "api",
		ComponentID:    wrapperspb.UInt64(789),
		Status:         RouteStatusFailed,
		LastUpdated:    time.Now(),
	}

	routeID4 := da.AddRoute(routeWithEndpoint)
	require.NotEmpty(t, routeID4, "AddRoute should handle DestEndpoint")

	retrievedEndpointRoute := da.GetRouteByID(routeID4)
	require.NotNil(t, retrievedEndpointRoute)
	assert.Equal(t, "external.service.com:8080", retrievedEndpointRoute.DestEndpoint)
	assert.Empty(t, retrievedEndpointRoute.DestNodeID, "DestNodeID should be empty when DestEndpoint is used")
}

// TestDataAccess_NodeOperations tests all node-related operations
func TestDataAccess_NodeOperations(t *testing.T) {
	implementations := getDataAccessImplementations()

	for _, impl := range implementations {
		t.Run(impl.Name, func(t *testing.T) {
			da := impl.Factory()
			defer impl.Cleanup(da)

			testNodeOperations(t, da)
		})
	}
}

func testNodeOperations(t *testing.T, da DataAccess) {
	groupName := "group1"
	// Test SaveNode - new node (empty ID)
	newNode := Node{
		ID:        "", // Empty ID for new node
		GroupName: &groupName,
		ConnDetails: []ConnectionDetails{
			{
				Endpoint:     "localhost:8080",
				MTLSRequired: true,
			},
		},
		LastUpdated: time.Now(),
	}

	nodeID, isNewOrChanged, err := da.SaveNode(newNode)
	require.NoError(t, err, "SaveNode should not return error for new node")
	require.NotEmpty(t, nodeID, "SaveNode should return non-empty node ID")
	assert.False(t, isNewOrChanged, "New node should return false for isNewOrChanged")

	// Test SaveNode - update existing node with same details
	existingNode := Node{
		ID:        nodeID,
		GroupName: &groupName,
		ConnDetails: []ConnectionDetails{
			{
				Endpoint:     "localhost:8080",
				MTLSRequired: true,
			},
		},
		LastUpdated: time.Now(),
	}

	returnedID, isNewOrChanged, err := da.SaveNode(existingNode)
	require.NoError(t, err, "SaveNode should not return error for existing node")
	assert.Equal(t, nodeID, returnedID, "SaveNode should return same node ID")
	assert.False(t, isNewOrChanged, "Unchanged node should return false for isNewOrChanged")

	// Test SaveNode - update existing node with changed connection details
	changedNode := Node{
		ID:        nodeID,
		GroupName: &groupName,
		ConnDetails: []ConnectionDetails{
			{
				Endpoint:     "localhost:8080",
				MTLSRequired: false, // Changed from true to false
			},
		},
		LastUpdated: time.Now(),
	}

	returnedID, isNewOrChanged, err = da.SaveNode(changedNode)
	require.NoError(t, err, "SaveNode should not return error for changed node")
	assert.Equal(t, nodeID, returnedID, "SaveNode should return same node ID")
	assert.True(t, isNewOrChanged, "Changed node should return true for isNewOrChanged")

	// Test SaveNode - update with additional connection details
	nodeWithMoreConnections := Node{
		ID:        nodeID,
		GroupName: &groupName,
		ConnDetails: []ConnectionDetails{
			{
				Endpoint:     "localhost:8080",
				MTLSRequired: false,
			},
			{
				Endpoint:     "localhost:9090", // Additional connection
				MTLSRequired: true,
			},
		},
		LastUpdated: time.Now(),
	}

	returnedID, isNewOrChanged, err = da.SaveNode(nodeWithMoreConnections)
	require.NoError(t, err, "SaveNode should not return error")
	assert.Equal(t, nodeID, returnedID, "SaveNode should return same node ID")
	assert.True(t, isNewOrChanged, "Node with additional connections should return true")

	// Test GetNode
	retrievedNode, err := da.GetNode(nodeID)
	require.NoError(t, err, "GetNode should not return error")
	require.NotNil(t, retrievedNode, "GetNode should return the node")
	assert.Equal(t, nodeID, retrievedNode.ID)
	assert.Equal(t, "group1", *retrievedNode.GroupName)
	assert.Len(t, retrievedNode.ConnDetails, 2, "Should have 2 connection details")

	// Test GetNode with non-existent ID
	nonExistentNode, err := da.GetNode("non-existent-id")
	assert.Error(t, err, "GetNode should return error for non-existent node")
	assert.Nil(t, nonExistentNode, "GetNode should return nil for non-existent node")

	// Test DeleteNode
	err = da.DeleteNode(nodeID)
	assert.NoError(t, err, "DeleteNode should not return error")

	deletedNode, err := da.GetNode(nodeID)
	assert.Error(t, err, "GetNode should return error for deleted node")
	assert.Nil(t, deletedNode, "Node should be completely removed")

	// Test DeleteNode with non-existent ID
	err = da.DeleteNode("non-existent-id")
	assert.Error(t, err, "DeleteNode should return error for non-existent node")
}

// TestDataAccess_ChannelOperations tests all channel-related operations
func TestDataAccess_ChannelOperations(t *testing.T) {
	implementations := getDataAccessImplementations()

	for _, impl := range implementations {
		t.Run(impl.Name, func(t *testing.T) {
			da := impl.Factory()
			defer impl.Cleanup(da)

			testChannelOperations(t, da)
		})
	}
}

func testChannelOperations(t *testing.T, da DataAccess) {
	// Test SaveChannel
	channelID := "org/ns/channel123"
	moderators := []string{"org/ns/mod1/0", "org/ns/mod2/0"}

	err := da.SaveChannel(channelID, moderators)
	assert.NoError(t, err, "SaveChannel should not return error")

	// Test GetChannel
	channel, err := da.GetChannel(channelID)
	assert.NoError(t, err, "GetChannel should not return error")
	assert.Equal(t, channelID, channel.ID, "Channel ID should match")
	assert.ElementsMatch(t, moderators, channel.Moderators, "Moderators should match")
	assert.Empty(t, channel.Participants, "New channel should have no participants")

	// Test GetChannel with non-existent ID
	_, err = da.GetChannel("non-existent-channel")
	assert.Error(t, err, "GetChannel should return error for non-existent channel")

	// Test UpdateChannel - add participants
	channel.Participants = []string{"org/ns/participant1/0", "org/ns/participant2/0"}
	err = da.UpdateChannel(channel)
	assert.NoError(t, err, "UpdateChannel should not return error")

	updatedChannel, err := da.GetChannel(channelID)
	require.NoError(t, err)
	assert.ElementsMatch(t, channel.Participants, updatedChannel.Participants, "Participants should be updated")

	// Test UpdateChannel with non-existent channel
	nonExistentChannel := Channel{
		ID:           "non-existent",
		Moderators:   []string{"mod"},
		Participants: []string{"participant"},
	}
	err = da.UpdateChannel(nonExistentChannel)
	assert.Error(t, err, "UpdateChannel should return error for non-existent channel")

	// Test ListChannels
	// Add another channel
	channel2ID := "org/ns/channel456"
	err = da.SaveChannel(channel2ID, []string{"org/ns/mod3/0"})
	require.NoError(t, err)

	channels, err := da.ListChannels()
	assert.NoError(t, err, "ListChannels should not return error")
	assert.GreaterOrEqual(t, len(channels), 2, "Should have at least 2 channels")

	// Verify our channels are in the list
	channelIDs := make([]string, len(channels))
	for i, ch := range channels {
		channelIDs[i] = ch.ID
	}
	assert.Contains(t, channelIDs, channelID, "Should contain first channel")
	assert.Contains(t, channelIDs, channel2ID, "Should contain second channel")

	// Test DeleteChannel
	err = da.DeleteChannel(channelID)
	assert.NoError(t, err, "DeleteChannel should not return error")

	_, err = da.GetChannel(channelID)
	assert.Error(t, err, "GetChannel should return error for deleted channel")

	// Test DeleteChannel with non-existent ID
	err = da.DeleteChannel("non-existent-channel")
	assert.Error(t, err, "DeleteChannel should return error for non-existent channel")
}

// TestDataAccess_ConcurrentOperations tests thread safety (if applicable)
func TestDataAccess_ConcurrentOperations(t *testing.T) {
	implementations := getDataAccessImplementations()

	for _, impl := range implementations {
		t.Run(impl.Name, func(t *testing.T) {
			da := impl.Factory()
			defer impl.Cleanup(da)

			testConcurrentOperations(t, da)
		})
	}
}

func testConcurrentOperations(t *testing.T, da DataAccess) {
	const numGoroutines = 10
	const operationsPerGoroutine = 100

	done := make(chan bool, numGoroutines)

	// Concurrent route operations
	for i := 0; i < numGoroutines; i++ {
		go func(workerID int) {
			defer func() { done <- true }()

			for j := 0; j < operationsPerGoroutine; j++ {
				route := Route{
					SourceNodeID: fmt.Sprintf("worker-%d", workerID),
					DestNodeID:   fmt.Sprintf("dest-%d-%d", workerID, j),
					Component0:   "org",
					Component1:   "service",
					Component2:   "method",
					ComponentID:  wrapperspb.UInt64(uint64(j)),
					Status:       RouteStatusFailed,
					LastUpdated:  time.Now(),
				}

				routeID := da.AddRoute(route)
				assert.NotEmpty(t, routeID)

				// Randomly update or delete some routes
				if j%3 == 0 {
					err := da.MarkRouteAsApplied(routeID)
					assert.NoError(t, err)
				} else if j%5 == 0 {
					err := da.DeleteRoute(routeID)
					assert.NoError(t, err)
				}
			}
		}(i)
	}

	// Wait for all goroutines to complete
	for i := 0; i < numGoroutines; i++ {
		select {
		case <-done:
			// Goroutine completed
		case <-time.After(30 * time.Second):
			t.Fatal("Timeout waiting for concurrent operations to complete")
		}
	}

	// Verify data integrity - should be able to query without panics
	routes := da.GetRoutesForNodeID("worker-0")
	assert.NotNil(t, routes) // Should not panic, even if empty
}

func BenchmarkDataAccess_AddRoute(b *testing.B) {
	implementations := getDataAccessImplementations()

	for _, impl := range implementations {
		b.Run(impl.Name, func(b *testing.B) {
			da := impl.Factory()
			defer impl.Cleanup(da)

			route := Route{
				SourceNodeID: "benchmark-source",
				DestNodeID:   "benchmark-dest",
				Component0:   "org",
				Component1:   "service",
				Component2:   "method",
				ComponentID:  wrapperspb.UInt64(123),
				Status:       RouteStatusFailed,
				LastUpdated:  time.Now(),
			}

			b.ResetTimer()

			for i := 0; i < b.N; i++ {
				route.DestNodeID = fmt.Sprintf("benchmark-dest-%d", i)
				_ = da.AddRoute(route)
			}
		})
	}
}

func BenchmarkDataAccess_GetRoutesForNodeID(b *testing.B) {
	implementations := getDataAccessImplementations()

	for _, impl := range implementations {
		b.Run(impl.Name, func(b *testing.B) {
			da := impl.Factory()
			defer impl.Cleanup(da)

			// Setup: Add some routes
			nodeID := "benchmark-node"
			for i := 0; i < 1000; i++ {
				route := Route{
					SourceNodeID: nodeID,
					DestNodeID:   fmt.Sprintf("dest-%d", i),
					Component0:   "org",
					Component1:   "service",
					Component2:   "method",
					ComponentID:  wrapperspb.UInt64(uint64(i)),
					Status:       RouteStatusFailed,
					LastUpdated:  time.Now(),
				}
				da.AddRoute(route)
			}

			b.ResetTimer()

			for i := 0; i < b.N; i++ {
				_ = da.GetRoutesForNodeID(nodeID)
			}
		})
	}
}
