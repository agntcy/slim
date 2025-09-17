package nbapiservice

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
	waitResp  *controllerapi.ControlMessage
}

type sendCall struct {
	nodeID string
	msg    *controllerapi.ControlMessage
}

func (m *CommandHandlerMock) SendMessage(ctx context.Context, nodeID string, msg *controllerapi.ControlMessage) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.sendCalls = append(m.sendCalls, sendCall{nodeID: nodeID, msg: msg})
	return nil
}
func (m *CommandHandlerMock) AddStream(ctx context.Context, nodeID string, stream controllerapi.ControllerService_OpenControlChannelServer) {
}
func (m *CommandHandlerMock) RemoveStream(ctx context.Context, nodeID string) error { return nil }
func (m *CommandHandlerMock) GetConnectionStatus(ctx context.Context, nodeID string) (nodecontrol.NodeStatus, error) {
	return nodecontrol.NodeStatusConnected, nil
}
func (m *CommandHandlerMock) UpdateConnectionStatus(ctx context.Context, nodeID string, status nodecontrol.NodeStatus) {
}
func (m *CommandHandlerMock) WaitForResponse(ctx context.Context, nodeID string, messageType reflect.Type, messageID string) (*controllerapi.ControlMessage, error) {
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
func (m *CommandHandlerMock) ResponseReceived(ctx context.Context, nodeID string, command *controllerapi.ControlMessage) {
}

func TestRouteService_NodeRegisteredAndAddRoute(t *testing.T) {
	ctx := util.GetContextWithLogger(context.Background(), config.LogConfig{
		Level: "debug",
	})
	dbService := db.NewInMemoryDBService()
	cmdHandler := &CommandHandlerMock{}

	routeService := NewRouteService(dbService, cmdHandler, 1)
	require.NoError(t, routeService.Start(ctx))

	// Add two nodes with connection details
	node1 := db.Node{
		ID: "node1",
		ConnDetails: []db.ConnectionDetails{
			{
				Endpoint:     "http://slim_node1_ip:1234",
				MtlsRequired: false,
			},
		},
	}
	node2 := db.Node{
		ID: "node2",
		ConnDetails: []db.ConnectionDetails{
			{
				Endpoint:     "http://slim_node2_ip:5678",
				MtlsRequired: false,
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

	// Add two routes with source '*' and dest node1/node2
	route1 := Route{
		SourceNodeID: node2.ID,
		DestNodeID:   node1.ID,
		Component0:   "comp0",
		Component1:   "comp1",
		Component2:   "comp2",
		ComponentID:  &wrapperspb.UInt64Value{Value: 1},
	}
	route2 := Route{
		SourceNodeID: node1.ID,
		DestNodeID:   node2.ID,
		Component0:   "comp0b",
		Component1:   "comp1b",
		Component2:   "comp2b",
		ComponentID:  &wrapperspb.UInt64Value{Value: 2},
	}
	_, err = routeService.AddRoute(ctx, route1)
	require.NoError(t, err)
	_, err = routeService.AddRoute(ctx, route2)
	require.NoError(t, err)

	require.NoError(t, routeService.Start(ctx))

	// Wait for goroutines to process the queue
	// (in real code, use sync or channels; here, just sleep briefly)
	time.Sleep(5 * time.Second) // Uncomment if needed

	// Check SendMessage was called for both nodes
	cmdHandler.mu.Lock()
	defer cmdHandler.mu.Unlock()
	require.GreaterOrEqual(t, len(cmdHandler.sendCalls), 2, "SendMessage should be called for both nodes")

	expectedSubscriptions := map[string][]string{
		"node1": {"comp0b/comp1b/comp2b"}, // node1 should subscribe to route1
		"node2": {"comp0/comp1/comp2"},    // node2 should subscribe to route2
	}

	// Collect nodeIDs for which SendMessage was called
	nodeIDs := map[string]bool{}
	for _, call := range cmdHandler.sendCalls {
		nodeIDs[call.nodeID] = true
		// Optionally, check the message content
		require.NotNil(t, call.msg)
		require.NotEmpty(t, call.msg.MessageId)
		require.NotNil(t, call.msg.GetConfigCommand())
		// Check that the config command contains the expected endpoint
		connections := call.msg.GetConfigCommand().ConnectionsToCreate
		require.NotEmpty(t, connections)
		for _, conn := range connections {
			var config map[string]interface{}
			err := json.Unmarshal([]byte(conn.ConfigData), &config)
			require.NoError(t, err)
			require.Contains(t, config["endpoint"], "slim_node")
		}
		// Check subscriptions in config command
		subscriptions := call.msg.GetConfigCommand().SubscriptionsToSet
		require.NotEmpty(t, subscriptions)
		var gotSubs []string
		for _, sub := range subscriptions {
			gotSubs = append(gotSubs, sub.Component_0+"/"+sub.Component_1+"/"+sub.Component_2)
		}
		require.ElementsMatch(t, expectedSubscriptions[call.nodeID], gotSubs, "subscriptions for %s do not match", call.nodeID)
	}
	require.True(t, nodeIDs["node1"], "SendMessage should be called for node1")
	require.True(t, nodeIDs["node2"], "SendMessage should be called for node2")
}
