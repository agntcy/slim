package routes

import (
	"context"
	"encoding/json"
	"fmt"
	"reflect"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	"k8s.io/client-go/util/workqueue"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
)

type testNodeCommandHandler struct {
	connectionStatus nodecontrol.NodeStatus
	connectionErr    error
	sendErr          error
	waitResp         *controllerapi.ControlMessage
	waitErr          error
	sent             []*controllerapi.ControlMessage
}

func (m *testNodeCommandHandler) SendMessage(
	_ context.Context, _ string, configurationCommand *controllerapi.ControlMessage) error {
	m.sent = append(m.sent, configurationCommand)
	return m.sendErr
}

func (m *testNodeCommandHandler) AddStream(
	context.Context, string, controllerapi.ControllerService_OpenControlChannelServer) {
}

func (m *testNodeCommandHandler) RemoveStream(context.Context, string) error { return nil }

func (m *testNodeCommandHandler) GetConnectionStatus(
	context.Context, string) (nodecontrol.NodeStatus, error) {
	return m.connectionStatus, m.connectionErr
}

func (m *testNodeCommandHandler) UpdateConnectionStatus(context.Context, string, nodecontrol.NodeStatus) {
}

func (m *testNodeCommandHandler) WaitForResponse(
	context.Context, string, reflect.Type, string) (*controllerapi.ControlMessage, error) {
	return m.waitResp, m.waitErr
}

func (m *testNodeCommandHandler) WaitForResponseWithTimeout(
	context.Context, string, reflect.Type, string, time.Duration) (*controllerapi.ControlMessage, error) {
	return m.waitResp, m.waitErr
}

func (m *testNodeCommandHandler) ResponseReceived(context.Context, string, *controllerapi.ControlMessage) {
}

func newLinkReconcilerForTest(
	dbService db.DataAccess,
	handler nodecontrol.NodeCommandHandler,
) (*LinkReconciler, workqueue.TypedRateLimitingInterface[RouteReconcileRequest]) {
	linkQueue := workqueue.NewTypedRateLimitingQueue(workqueue.DefaultTypedControllerRateLimiter[LinkReconcileRequest]())
	routeQueue := workqueue.NewTypedRateLimitingQueue(workqueue.DefaultTypedControllerRateLimiter[RouteReconcileRequest]())

	reconciler := NewLinkReconciler(
		"test-link-reconciler",
		config.ReconcilerConfig{MaxNumOfParallelReconciles: 1, MaxRequeues: 0},
		linkQueue,
		routeQueue,
		dbService,
		handler,
	)
	return reconciler, routeQueue
}

func TestLinkReconciler_HandleRequest_AppliesCreatesAndDeletesAckedLinks(t *testing.T) {
	ctx := context.Background()
	dbService := db.NewInMemoryDBService()

	activeLink := db.Link{
		LinkID:         "link-create",
		SourceNodeID:   "node-a",
		DestNodeID:     "node-b",
		DestEndpoint:   "http://node-b:8080",
		ConnConfigData: `{"endpoint":"http://node-b:8080"}`,
		Status:         db.LinkStatusPending,
	}
	_, err := dbService.AddLink(activeLink)
	require.NoError(t, err)

	deletedLink := db.Link{
		LinkID:         "link-delete",
		SourceNodeID:   "node-a",
		DestNodeID:     "node-c",
		DestEndpoint:   "http://node-c:8080",
		ConnConfigData: `{"endpoint":"http://node-c:8080"}`,
		Status:         db.LinkStatusPending,
		Deleted:        true,
	}
	_, err = dbService.AddLink(deletedLink)
	require.NoError(t, err)

	_, err = dbService.AddRoute(db.Route{
		SourceNodeID: "node-a",
		DestNodeID:   "node-b",
		LinkID:       "link-create",
		Component0:   "org",
		Component1:   "ns",
		Component2:   "svc",
		Status:       db.RouteStatusPending,
	})
	require.NoError(t, err)

	handler := &testNodeCommandHandler{
		connectionStatus: nodecontrol.NodeStatusConnected,
		waitResp: &controllerapi.ControlMessage{
			Payload: &controllerapi.ControlMessage_ConfigCommandAck{
				ConfigCommandAck: &controllerapi.ConfigurationCommandAck{
					ConnectionsStatus: []*controllerapi.ConnectionAck{
						{ConnectionId: "link-create", Success: true},
						{ConnectionId: "link-delete", Success: true},
					},
				},
			},
		},
	}
	reconciler, routeQueue := newLinkReconcilerForTest(dbService, handler)
	t.Cleanup(func() {
		routeQueue.ShutDown()
	})

	err = reconciler.handleRequest(ctx, LinkReconcileRequest{NodeID: "node-a"})
	require.NoError(t, err)
	require.Len(t, handler.sent, 1)

	cfg := handler.sent[0].GetConfigCommand()
	require.NotNil(t, cfg)
	require.ElementsMatch(t, []string{"link-delete"}, cfg.GetConnectionsToDelete())
	require.Len(t, cfg.GetConnectionsToCreate(), 1)
	require.Equal(t, "link-create", cfg.GetConnectionsToCreate()[0].GetConnectionId())

	var cfgMap map[string]any
	err = json.Unmarshal([]byte(cfg.GetConnectionsToCreate()[0].GetConfigData()), &cfgMap)
	require.NoError(t, err)
	require.Equal(t, "link-create", cfgMap["link_id"])

	updatedActive, err := dbService.GetLink("link-create", "node-a", "node-b")
	require.NoError(t, err)
	require.Equal(t, db.LinkStatusApplied, updatedActive.Status)
	require.Empty(t, updatedActive.StatusMsg)

	_, err = dbService.GetLink("link-delete", "node-a", "node-c")
	require.Error(t, err, "deleted link must be removed after successful delete ack")

	require.Equal(t, 1, routeQueue.Len(), "reconciler should enqueue source node once")
}

func TestLinkReconciler_HandleRequest_FailsCreateAndMissingDeleteAck(t *testing.T) {
	ctx := context.Background()
	dbService := db.NewInMemoryDBService()

	_, err := dbService.AddLink(db.Link{
		LinkID:         "link-fail-create",
		SourceNodeID:   "node-a",
		DestNodeID:     "node-b",
		DestEndpoint:   "http://node-b:8080",
		ConnConfigData: `{"endpoint":"http://node-b:8080","link_id":"link-fail-create"}`,
		Status:         db.LinkStatusPending,
	})
	require.NoError(t, err)

	_, err = dbService.AddLink(db.Link{
		LinkID:         "link-delete-missing",
		SourceNodeID:   "node-a",
		DestNodeID:     "node-c",
		DestEndpoint:   "http://node-c:8080",
		ConnConfigData: `{"endpoint":"http://node-c:8080"}`,
		Status:         db.LinkStatusPending,
		Deleted:        true,
	})
	require.NoError(t, err)

	handler := &testNodeCommandHandler{
		connectionStatus: nodecontrol.NodeStatusConnected,
		waitResp: &controllerapi.ControlMessage{
			Payload: &controllerapi.ControlMessage_ConfigCommandAck{
				ConfigCommandAck: &controllerapi.ConfigurationCommandAck{
					ConnectionsStatus: []*controllerapi.ConnectionAck{
						{
							ConnectionId: "link-fail-create",
							Success:      false,
							ErrorMsg:     "create failed",
						},
					},
				},
			},
		},
	}
	reconciler, routeQueue := newLinkReconcilerForTest(dbService, handler)
	t.Cleanup(func() {
		routeQueue.ShutDown()
	})

	err = reconciler.handleRequest(ctx, LinkReconcileRequest{NodeID: "node-a"})
	require.Error(t, err)
	require.Contains(t, err.Error(), "link-delete-missing")

	failedCreate, err := dbService.GetLink("link-fail-create", "node-a", "node-b")
	require.NoError(t, err)
	require.Equal(t, db.LinkStatusFailed, failedCreate.Status)
	require.Equal(t, "create failed", failedCreate.StatusMsg)

	links := dbService.GetLinksForNode("node-a")
	var failedDelete *db.Link
	for _, link := range links {
		if link.LinkID == "link-delete-missing" {
			candidate := link
			failedDelete = &candidate
			break
		}
	}
	require.NotNil(t, failedDelete)
	require.True(t, failedDelete.Deleted)
	require.Equal(t, db.LinkStatusFailed, failedDelete.Status)
	require.Equal(t, "missing delete ack", failedDelete.StatusMsg)
}

func TestLinkReconciler_HandleRequest_ReturnsEarlyWhenNodeDisconnected(t *testing.T) {
	ctx := context.Background()
	dbService := db.NewInMemoryDBService()
	handler := &testNodeCommandHandler{connectionStatus: nodecontrol.NodeStatusNotConnected}
	reconciler, routeQueue := newLinkReconcilerForTest(dbService, handler)
	t.Cleanup(func() {
		routeQueue.ShutDown()
	})

	err := reconciler.handleRequest(ctx, LinkReconcileRequest{NodeID: "node-a"})
	require.NoError(t, err)
	require.Empty(t, handler.sent, "no command should be sent when node is disconnected")
	require.Equal(t, 0, routeQueue.Len(), "no route reconcile should be queued on early return")
}

func TestLinkReconciler_HandleRequest_RejectsInvalidAckPayload(t *testing.T) {
	ctx := context.Background()
	dbService := db.NewInMemoryDBService()

	_, err := dbService.AddLink(db.Link{
		LinkID:         "link-1",
		SourceNodeID:   "node-a",
		DestNodeID:     "node-b",
		DestEndpoint:   "http://node-b:8080",
		ConnConfigData: `{"endpoint":"http://node-b:8080"}`,
		Status:         db.LinkStatusPending,
	})
	require.NoError(t, err)

	handler := &testNodeCommandHandler{
		connectionStatus: nodecontrol.NodeStatusConnected,
		waitResp: &controllerapi.ControlMessage{
			Payload: &controllerapi.ControlMessage_Ack{
				Ack: &controllerapi.Ack{Success: true},
			},
		},
	}
	reconciler, routeQueue := newLinkReconcilerForTest(dbService, handler)
	t.Cleanup(func() {
		routeQueue.ShutDown()
	})

	err = reconciler.handleRequest(ctx, LinkReconcileRequest{NodeID: "node-a"})
	require.Error(t, err)
	require.Contains(t, err.Error(), fmt.Sprintf("received invalid ConfigCommandAck response from node %s", "node-a"))
}
