package routes

import (
	"context"
	"encoding/json"
	"reflect"
	"sync"
	"testing"
	"time"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
	"github.com/agntcy/slim/control-plane/control-plane/internal/util"

	"github.com/stretchr/testify/require"
	"google.golang.org/protobuf/types/known/wrapperspb"
)

// CommandHandlerMock is a mock for NodeCommandHandler
type CommandHandlerMock struct {
	mu        sync.Mutex
	sendCalls []sendCall
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
	ctx := util.GetContextWithLogger(context.Background(), config.LogConfig{
		Level: "debug",
	})
	dbService := db.NewInMemoryDBService()
	cmdHandler := &CommandHandlerMock{}

	routeService := NewRouteService(dbService, cmdHandler, 1)

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
				Endpoint:     "http://slim_node1_ip:1234",
				MTLSRequired: false,
			},
		},
	}
	node2 := db.Node{
		ID: "node2",
		ConnDetails: []db.ConnectionDetails{
			{
				Endpoint:     "http://slim_node2_ip:5678",
				MTLSRequired: false,
			},
		},
	}
	_, err := dbService.SaveNode(node1)
	require.NoError(t, err)
	_, err = dbService.SaveNode(node2)
	require.NoError(t, err)

	// Call NodeRegistered for each node
	routeService.NodeRegistered(ctx, node1.ID)
	routeService.NodeRegistered(ctx, node2.ID)
}

func TestRouteService_AddAndThenDeleteRoutes(t *testing.T) {
	ctx := util.GetContextWithLogger(context.Background(), config.LogConfig{
		Level: "debug",
	})
	dbService := db.NewInMemoryDBService()
	cmdHandler := &CommandHandlerMock{}

	routeService := NewRouteService(dbService, cmdHandler, 1)

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
	routeService := NewRouteService(dbService, cmdHandler, 1)

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
	routeService := NewRouteService(dbService, cmdHandler, 1)

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
	routeService := NewRouteService(dbService, cmdHandler, 1)

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
