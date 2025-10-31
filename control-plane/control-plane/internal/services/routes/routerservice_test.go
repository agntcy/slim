package routes

import (
	"context"
	"encoding/json"
	"fmt"
	"reflect"
	"sync"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	"google.golang.org/protobuf/types/known/wrapperspb"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
	"github.com/agntcy/slim/control-plane/control-plane/internal/util"
)

// CommandHandlerMock is a mock for NodeCommandHandler
type CommandHandlerMock struct {
	mu        sync.Mutex
	sendCalls []sendCall
	delay     int // milliseconds
}

type sendCall struct {
	nodeID string
	msg    *controllerapi.ControlMessage
}

func (m *CommandHandlerMock) SendMessage(_ context.Context, nodeID string, msg *controllerapi.ControlMessage) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.sendCalls = append(m.sendCalls, sendCall{nodeID: nodeID, msg: msg})
	return nil
}
func (m *CommandHandlerMock) AddStream(
	_ context.Context, _ string,
	_ controllerapi.ControllerService_OpenControlChannelServer) {
}
func (m *CommandHandlerMock) RemoveStream(_ context.Context, _ string) error { return nil }
func (m *CommandHandlerMock) GetConnectionStatus(
	_ context.Context, _ string) (nodecontrol.NodeStatus, error) {
	return nodecontrol.NodeStatusConnected, nil
}
func (m *CommandHandlerMock) UpdateConnectionStatus(_ context.Context,
	_ string, _ nodecontrol.NodeStatus) {
}
func (m *CommandHandlerMock) WaitForResponse(_ context.Context,
	_ string, _ reflect.Type, messageID string) (*controllerapi.ControlMessage, error) {
	if m.delay > 0 {
		time.Sleep(time.Duration(m.delay) * time.Millisecond)
	}
	// Always return a successful ACK
	return &controllerapi.ControlMessage{
		Payload: &controllerapi.ControlMessage_Ack{
			Ack: &controllerapi.Ack{
				Success:           true,
				OriginalMessageId: messageID,
			},
		},
	}, nil
}
func (m *CommandHandlerMock) WaitForResponseWithTimeout(_ context.Context,
	_ string, _ reflect.Type, messageID string, _ time.Duration) (*controllerapi.ControlMessage, error) {
	if m.delay > 0 {
		time.Sleep(time.Duration(m.delay) * time.Millisecond)
	}
	// Always return a successful ACK
	return &controllerapi.ControlMessage{
		Payload: &controllerapi.ControlMessage_Ack{
			Ack: &controllerapi.Ack{
				Success:           true,
				OriginalMessageId: messageID,
			},
		},
	}, nil
}
func (m *CommandHandlerMock) ResponseReceived(
	_ context.Context, _ string, _ *controllerapi.ControlMessage) {
}

// Reset clears the sendCalls array.
func (m *CommandHandlerMock) Reset() {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.sendCalls = make([]sendCall, 0)
}

func TestRouteService_AddRoutes(t *testing.T) {
	rConfig := config.ReconcilerConfig{
		MaxNumOfParallelReconciles: 10,
		MaxRequeues:                0,
	}

	ctx := util.GetContextWithLogger(context.Background(), config.LogConfig{
		Level: "debug",
	})
	dbService := db.NewInMemoryDBService()
	cmdHandler := &CommandHandlerMock{}

	routeService := NewRouteService(dbService, cmdHandler, rConfig)

	addNodes(ctx, t, dbService, routeService)

	// Add two routes with source '*' and dest node1/node2
	route1 := Route{
		SourceNodeID: "node2",
		DestNodeID:   "node1",
		Component0:   "org",
		Component1:   "ns",
		Component2:   "client_1",
		ComponentID:  &wrapperspb.UInt64Value{Value: 1},
	}
	_, err := routeService.AddRoute(ctx, route1)
	require.NoError(t, err)

	route2 := Route{
		SourceNodeID:   "node1",
		DestEndpoint:   "http://slim_node2_ip:5678",
		ConnConfigData: "{\"endpoint\":\"http://slim_node2_ip:5678\"}", // using endpoint instead of nodeID
		Component0:     "org",
		Component1:     "ns",
		Component2:     "client_2",
		ComponentID:    &wrapperspb.UInt64Value{Value: 2},
	}
	_, err = routeService.AddRoute(ctx, route2)
	require.NoError(t, err)

	genericRoute := Route{
		SourceNodeID: AllNodesID,
		DestNodeID:   "node1",
		Component0:   "org",
		Component1:   "ns",
		Component2:   "client_3",
		ComponentID:  &wrapperspb.UInt64Value{Value: 2},
	}
	_, err = routeService.AddRoute(ctx, genericRoute)
	require.NoError(t, err)

	require.NoError(t, routeService.Start(ctx))
	// Wait for goroutines to process the queue
	// (in real code, use sync or channels; here, just sleep briefly)
	time.Sleep(5 * time.Second) // Uncomment if needed

	require.GreaterOrEqual(t, len(cmdHandler.sendCalls),
		2, "SendMessage should be called for both nodes")

	expectedConnectionEndpoints := map[string][]string{
		"node1": {"http://slim_node2_ip:5678"}, // node1 should be connected to node2
		"node2": {"http://slim_node1_ip:1234"}, // node2 should be connected to node1
	}
	expectedSubscriptions := map[string][]string{
		"node1": {"org/ns/client_2"},                    // node1 should subscribe to route1
		"node2": {"org/ns/client_1", "org/ns/client_3"}, // node2 should subscribe to route2
	}
	expectedSubscriptionsToDelete := map[string][]string{
		"node1": {},
		"node2": {},
	}
	assertConnsAndSubs(t, cmdHandler,
		expectedConnectionEndpoints, expectedSubscriptions, expectedSubscriptionsToDelete)
}

func addNodes(ctx context.Context, t *testing.T, dbService db.DataAccess, routeService *RouteService) {

	// Add two nodes with connection details
	node1 := db.Node{
		ID: "node1",
		ConnDetails: []db.ConnectionDetails{
			{
				Endpoint:     "slim_node1_ip:1234",
				MTLSRequired: false,
			},
		},
	}
	node2 := db.Node{
		ID: "node2",
		ConnDetails: []db.ConnectionDetails{
			{
				Endpoint:     "slim_node2_ip:5678",
				MTLSRequired: false,
			},
		},
	}
	_, _, err := dbService.SaveNode(node1)
	require.NoError(t, err)
	_, _, err = dbService.SaveNode(node2)
	require.NoError(t, err)

	// Call NodeRegistered for each node
	routeService.NodeRegistered(ctx, node1.ID, false)
	routeService.NodeRegistered(ctx, node2.ID, false)
}

func TestRouteService_AddAndThenDeleteRoutes(t *testing.T) {
	ctx := util.GetContextWithLogger(context.Background(), config.LogConfig{
		Level: "debug",
	})
	rConfig := config.ReconcilerConfig{
		MaxNumOfParallelReconciles: 10,
		MaxRequeues:                0,
	}
	dbService := db.NewInMemoryDBService()
	cmdHandler := &CommandHandlerMock{}

	routeService := NewRouteService(dbService, cmdHandler, rConfig)

	addNodes(ctx, t, dbService, routeService)

	// Add two routes with source '*' and dest node1/node2
	route1 := Route{
		SourceNodeID: "node2",
		DestNodeID:   "node1",
		Component0:   "org",
		Component1:   "ns",
		Component2:   "client_1",
		ComponentID:  &wrapperspb.UInt64Value{Value: 1},
	}
	_, err := routeService.AddRoute(ctx, route1)
	require.NoError(t, err)

	route2 := Route{
		SourceNodeID:   "node1",
		DestEndpoint:   "http://slim_node2_ip:5678",
		ConnConfigData: "{\"endpoint\":\"http://slim_node2_ip:5678\"}", // using endpoint instead of nodeID
		Component0:     "org",
		Component1:     "ns",
		Component2:     "client_2",
		ComponentID:    &wrapperspb.UInt64Value{Value: 2},
	}
	_, err = routeService.AddRoute(ctx, route2)
	require.NoError(t, err)

	genericRoute := Route{
		SourceNodeID: AllNodesID,
		DestNodeID:   "node1",
		Component0:   "org",
		Component1:   "ns",
		Component2:   "client_3",
		ComponentID:  &wrapperspb.UInt64Value{Value: 2},
	}
	_, err = routeService.AddRoute(ctx, genericRoute)
	require.NoError(t, err)

	err = routeService.DeleteRoute(ctx, route1)
	require.NoError(t, err)
	err = routeService.DeleteRoute(ctx, route2)
	require.NoError(t, err)
	err = routeService.DeleteRoute(ctx, genericRoute)
	require.NoError(t, err)

	require.NoError(t, routeService.Start(ctx))
	// Wait for goroutines to process the queue
	time.Sleep(3 * time.Second) // Uncomment if needed

	require.GreaterOrEqual(t, len(cmdHandler.sendCalls),
		2, "SendMessage should be called for both nodes after deletions")
	expectedConnectionEndpoints := map[string][]string{
		"node1": {}, // node1 should have no connections
		"node2": {}, // node2 should have no connections
	}
	expectedSubscriptions := map[string][]string{
		"node1": {}, // node1 should have no subscriptions
		"node2": {}, // node2 should have no subscriptions
	}
	expectedSubscriptionsToDelete := map[string][]string{
		"node1": {"org/ns/client_2"},                    // node1 should delete subscription to route1
		"node2": {"org/ns/client_1", "org/ns/client_3"}, // node2 should delete subscription to route2
	}
	assertConnsAndSubs(t, cmdHandler,
		expectedConnectionEndpoints, expectedSubscriptions, expectedSubscriptionsToDelete)
}

func assertConnsAndSubs(t *testing.T, cmdHandler *CommandHandlerMock,
	expectedConnectionEndpoints map[string][]string,
	expectedSubscriptions map[string][]string,
	expectedSubscriptionsToDelete map[string][]string) {

	for _, call := range cmdHandler.sendCalls {
		// Optionally, check the message content
		require.NotNil(t, call.msg)
		require.NotEmpty(t, call.msg.MessageId)
		require.NotNil(t, call.msg.GetConfigCommand())
		// Check that the config command contains the expected endpoint
		connections := call.msg.GetConfigCommand().ConnectionsToCreate
		var gotConns []string
		for _, conn := range connections {
			var config map[string]interface{}
			err := json.Unmarshal([]byte(conn.ConfigData), &config)
			require.NoError(t, err)
			require.Contains(t, config["endpoint"], "slim_node")
			gotConns = append(gotConns, config["endpoint"].(string))
		}
		require.ElementsMatch(t, expectedConnectionEndpoints[call.nodeID],
			gotConns, "connections for %s do not match", call.nodeID)

		// Check subscriptions in config command
		subscriptions := call.msg.GetConfigCommand().SubscriptionsToSet
		var gotSubs []string
		for _, sub := range subscriptions {
			gotSubs = append(gotSubs, sub.Component_0+"/"+sub.Component_1+"/"+sub.Component_2)
		}
		require.ElementsMatch(t, expectedSubscriptions[call.nodeID],
			gotSubs, "subscriptions for %s do not match", call.nodeID)

		// Check subscriptions to delete in config command
		subsToDelete := call.msg.GetConfigCommand().SubscriptionsToDelete
		var gotSubsToDelete []string
		for _, sub := range subsToDelete {
			gotSubsToDelete = append(gotSubsToDelete, sub.Component_0+"/"+sub.Component_1+"/"+sub.Component_2)
		}
		require.ElementsMatch(t, expectedSubscriptionsToDelete[call.nodeID],
			gotSubsToDelete, "subscriptions to delete for %s do not match", call.nodeID)
	}
}

// Add to internal/services/nbapiservice/routerservice_test.go

func TestRouteService_AddRoute_Validation(t *testing.T) {
	ctx := context.Background()
	dbService := db.NewInMemoryDBService()
	cmdHandler := &CommandHandlerMock{}
	rConfig := config.ReconcilerConfig{
		MaxNumOfParallelReconciles: 10,
		MaxRequeues:                0,
	}
	routeService := NewRouteService(dbService, cmdHandler, rConfig)

	route := Route{
		SourceNodeID: "node1",
		// Both DestNodeID and DestEndpoint are empty
		Component0:  "org",
		Component1:  "ns",
		Component2:  "client",
		ComponentID: &wrapperspb.UInt64Value{Value: 1},
	}
	_, err := routeService.AddRoute(ctx, route)
	require.Error(t, err)
	require.Contains(t, err.Error(),
		"either destNodeID or both destEndpoint and connConfigData must be set")
}

func TestRouteService_AddRoute_SameSourceAndDestValidation(t *testing.T) {
	ctx := context.Background()
	dbService := db.NewInMemoryDBService()
	cmdHandler := &CommandHandlerMock{}
	rConfig := config.ReconcilerConfig{
		MaxNumOfParallelReconciles: 10,
		MaxRequeues:                0,
	}
	routeService := NewRouteService(dbService, cmdHandler, rConfig)

	route := Route{
		SourceNodeID: "node1",
		DestNodeID:   "node1", // Same as source
		Component0:   "org",
		Component1:   "ns",
		Component2:   "client",
		ComponentID:  &wrapperspb.UInt64Value{Value: 1},
	}
	_, err := routeService.AddRoute(ctx, route)
	require.Error(t, err)
	require.Contains(t, err.Error(), "destination node ID cannot be the same as source node ID")
}

func TestRouteService_DeleteRoute_Validation(t *testing.T) {
	ctx := context.Background()
	dbService := db.NewInMemoryDBService()
	cmdHandler := &CommandHandlerMock{}
	rConfig := config.ReconcilerConfig{
		MaxNumOfParallelReconciles: 10,
		MaxRequeues:                0,
	}
	routeService := NewRouteService(dbService, cmdHandler, rConfig)

	route := Route{
		SourceNodeID: "node1",
		// Both DestNodeID and DestEndpoint are empty
		Component0:  "org",
		Component1:  "ns",
		Component2:  "client",
		ComponentID: &wrapperspb.UInt64Value{Value: 1},
	}
	err := routeService.DeleteRoute(ctx, route)
	require.Error(t, err)
	require.Contains(t, err.Error(), "either destNodeID or both destEndpoint must be set")
}

func TestSelectConnection(t *testing.T) {
	tests := []struct {
		name          string
		dstNode       *db.Node
		srcNode       *db.Node
		expectedConn  db.ConnectionDetails
		expectedLocal bool
		description   string
	}{
		{
			name: "same_group_names_both_non_nil",
			dstNode: &db.Node{
				GroupName: stringPtr("group1"),
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1"},
					{Endpoint: "dst2"},
				},
			},
			srcNode: &db.Node{
				GroupName: stringPtr("group1"),
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1"},
			expectedLocal: true,
			description:   "same group names should return first connection as local",
		},
		{
			name: "different_group_names_with_external_endpoint",
			dstNode: &db.Node{
				GroupName: stringPtr("group1"),
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1", ExternalEndpoint: nil},
					{Endpoint: "dst2", ExternalEndpoint: stringPtr("external2")},
				},
			},
			srcNode: &db.Node{
				GroupName: stringPtr("group2"),
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst2", ExternalEndpoint: stringPtr("external2")},
			expectedLocal: false,
			description:   "different groups should return first connection with external endpoint",
		},
		{
			name: "different_group_names_no_external_endpoint",
			dstNode: &db.Node{
				GroupName: stringPtr("group1"),
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1"},
					{Endpoint: "dst2"},
				},
			},
			srcNode: &db.Node{
				GroupName: stringPtr("group2"),
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1"},
			expectedLocal: false,
			description:   "different groups with no external endpoint should return first connection",
		},
		{
			name: "dst_group_nil_src_group_non_nil",
			dstNode: &db.Node{
				GroupName: nil,
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1"},
				},
			},
			srcNode: &db.Node{
				GroupName: stringPtr("group1"),
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1"},
			expectedLocal: false,
			description:   "dst nil group with src non-nil group should be external",
		},
		{
			name: "dst_group_non_nil_src_group_nil",
			dstNode: &db.Node{
				GroupName: stringPtr("group1"),
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1"},
				},
			},
			srcNode: &db.Node{
				GroupName: nil,
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1"},
			expectedLocal: false,
			description:   "dst non-nil group with src nil group should be external",
		},
		{
			name: "both_groups_nil",
			dstNode: &db.Node{
				GroupName: nil,
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1"},
				},
			},
			srcNode: &db.Node{
				GroupName: nil,
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1"},
			expectedLocal: true,
			description:   "both nil groups should be local",
		},
		{
			name: "external_endpoint_empty_string_skip_to_next",
			dstNode: &db.Node{
				GroupName: stringPtr("group1"),
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1", ExternalEndpoint: stringPtr("")},
					{Endpoint: "dst2", ExternalEndpoint: stringPtr("external2")},
				},
			},
			srcNode: &db.Node{
				GroupName: stringPtr("group2"),
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst2", ExternalEndpoint: stringPtr("external2")},
			expectedLocal: false,
			description:   "empty external endpoint should be skipped for next valid one",
		},
		{
			name: "all_external_endpoints_empty_fallback_to_first",
			dstNode: &db.Node{
				GroupName: stringPtr("group1"),
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1", ExternalEndpoint: stringPtr("")},
					{Endpoint: "dst2", ExternalEndpoint: nil},
				},
			},
			srcNode: &db.Node{
				GroupName: stringPtr("group2"),
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1", ExternalEndpoint: stringPtr("")},
			expectedLocal: false,
			description:   "all empty/nil external endpoints should fallback to first connection",
		},
		{
			name: "same_group_names_with_external_endpoints_ignored",
			dstNode: &db.Node{
				GroupName: stringPtr("group1"),
				ConnDetails: []db.ConnectionDetails{
					{Endpoint: "dst1", ExternalEndpoint: stringPtr("external1")},
				},
			},
			srcNode: &db.Node{
				GroupName: stringPtr("group1"),
			},
			expectedConn:  db.ConnectionDetails{Endpoint: "dst1", ExternalEndpoint: stringPtr("external1")},
			expectedLocal: true,
			description:   "same groups should ignore external endpoints and return first connection",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotConn, gotLocal := selectConnection(tt.dstNode, tt.srcNode)
			require.Equal(t, tt.expectedConn, gotConn, tt.description)
			require.Equal(t, tt.expectedLocal, gotLocal, tt.description)
		})
	}
}

// getSendCallsForNode returns all sendCalls for a specific nodeID
func getSendCallsForNode(cmdHandler *CommandHandlerMock, nodeID string) []sendCall {
	cmdHandler.mu.Lock()
	defer cmdHandler.mu.Unlock()

	var calls []sendCall
	for _, call := range cmdHandler.sendCalls {
		if call.nodeID == nodeID {
			calls = append(calls, call)
		}
	}

	return calls
}

func TestRouteReconciler_SameNodeIDSerialProcessing(t *testing.T) {
	ctx := util.GetContextWithLogger(context.Background(), config.LogConfig{
		Level: "debug",
	})
	rConfig := config.ReconcilerConfig{
		MaxNumOfParallelReconciles: 1000, // High limit to focus on same NodeID serialization
		MaxRequeues:                0,
	}
	dbService := db.NewInMemoryDBService()
	cmdHandler := &CommandHandlerMock{
		delay: 3000, // 3000ms delay to simulate processing time
	}

	routeService := NewRouteService(dbService, cmdHandler, rConfig)
	addNodes(ctx, t, dbService, routeService)

	// Add multiple routes for the same node
	route1 := Route{
		SourceNodeID: "node1",
		DestNodeID:   "node2",
		Component0:   "org",
		Component1:   "ns",
		Component2:   "client_1",
		ComponentID:  &wrapperspb.UInt64Value{Value: 1},
	}
	route2 := Route{
		SourceNodeID: "node2",
		DestNodeID:   "node1",
		Component0:   "org",
		Component1:   "ns",
		Component2:   "client_2",
		ComponentID:  &wrapperspb.UInt64Value{Value: 2},
	}

	require.NoError(t, routeService.Start(ctx))

	_, err := routeService.AddRoute(ctx, route1)
	require.NoError(t, err)
	// Wait for processing to complete
	time.Sleep(100 * time.Millisecond)
	node1Calls := getSendCallsForNode(cmdHandler, "node1")
	require.Equal(t, 1, len(node1Calls), "node1 should have received only one call")

	_, err = routeService.AddRoute(ctx, route2)
	require.NoError(t, err)
	// Wait for processing to complete
	time.Sleep(100 * time.Millisecond)
	node2Calls := getSendCallsForNode(cmdHandler, "node2")
	require.Equal(t, 1, len(node2Calls), "node2 should have received only one call")

	// trigger reconciles for node1 while the first is still processing
	var i uint64
	for i = 3; i < 13; i++ {
		_, err = routeService.AddRoute(ctx, Route{
			SourceNodeID: "node1",
			DestNodeID:   "node2",
			Component0:   "org",
			Component1:   "ns",
			Component2:   fmt.Sprintf("client_%d", i),
			ComponentID:  &wrapperspb.UInt64Value{Value: i},
		})
		require.NoError(t, err)
	}

	// Wait for processing to complete
	time.Sleep(4 * time.Second)
	node1Calls = getSendCallsForNode(cmdHandler, "node1")
	require.Equal(t, 2, len(node1Calls), "node1 should have received only one more call")
}

func TestRouteReconciler_MaxNumOfParallelReconciles(t *testing.T) {
	ctx := util.GetContextWithLogger(context.Background(), config.LogConfig{
		Level: "debug",
	})
	rConfig := config.ReconcilerConfig{
		MaxNumOfParallelReconciles: 1, // High limit to focus on same NodeID serialization
		MaxRequeues:                0,
	}
	dbService := db.NewInMemoryDBService()
	cmdHandler := &CommandHandlerMock{
		delay: 3000, // 3000ms delay to simulate processing time
	}

	routeService := NewRouteService(dbService, cmdHandler, rConfig)
	addNodes(ctx, t, dbService, routeService)

	// Add multiple routes for the same node
	route2 := Route{
		SourceNodeID: "node2",
		DestNodeID:   "node1",
		Component0:   "org",
		Component1:   "ns",
		Component2:   "client_2",
		ComponentID:  &wrapperspb.UInt64Value{Value: 2},
	}

	require.NoError(t, routeService.Start(ctx))

	_, err := routeService.AddRoute(ctx, route2)
	require.NoError(t, err)
	// Wait for processing to complete
	time.Sleep(5100 * time.Millisecond)
	node2Calls := getSendCallsForNode(cmdHandler, "node2")
	require.Equal(t, 1, len(node2Calls), "node2 should have received only one call")

}
