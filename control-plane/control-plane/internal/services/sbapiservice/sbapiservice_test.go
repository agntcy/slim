package sbapiservice

import (
	"context"
	"fmt"
	"net"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/groupservice"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/routes"
	"github.com/agntcy/slim/control-plane/control-plane/internal/util"
)

// startSouthbound spins up a grpc server with Southbound API and returns listen target.
func startSouthbound(t *testing.T, db db.DataAccess) (target string, cleanup func()) {
	// t.Helper()

	ctx := util.GetContextWithLogger(context.Background(), config.LogConfig{Level: "debug"})
	cmdHandler := nodecontrol.DefaultNodeCommandHandler()
	routeService := routes.NewRouteService(db, cmdHandler,
		config.ReconcilerConfig{MaxNumOfParallelReconciles: 3, MaxRequeues: 1})
	if err := routeService.Start(ctx); err != nil {
		t.Fatalf("routeService.Start: %v", err)
	}
	grp := groupservice.NewGroupService(db, cmdHandler)

	s := grpc.NewServer(grpc.Creds(insecure.NewCredentials()))
	svc := NewSBAPIService(config.APIConfig{
		HTTPHost: "127.0.0.1",
		HTTPPort: "50052",
	}, config.LogConfig{Level: "debug"}, db, cmdHandler, routeService, grp)
	controllerapi.RegisterControllerServiceServer(s, svc)
	l, err := net.Listen("tcp", "127.0.0.1:50052")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	go func() { _ = s.Serve(l) }()
	return l.Addr().String(), func() { s.Stop(); l.Close(); routeService.Stop() }
}

func waitCond(t *testing.T, d time.Duration, cond func() bool, msg string) {
	fmt.Println(msg)
	deadline := time.Now().Add(d)
	for time.Now().Before(deadline) {
		time.Sleep(1 * time.Second)
		if cond() {
			return
		}
	}
	t.Fatalf("condition `%s` not met within %s", msg, d)
}

func TestSouthbound_RegistrationAndRouteHandling(t *testing.T) {
	db := db.NewInMemoryDBService()
	target, cleanup := startSouthbound(t, db)
	defer cleanup()

	// start three mock nodes
	slim0, _ := NewMockSlimServer("slim-0", 4500, target)
	slim1, _ := NewMockSlimServer("slim-1", 4501, target)
	slim2, _ := NewMockSlimServer("slim-2", 4502, target)

	ctx := context.Background()
	if err := slim0.Start(ctx); err != nil {
		t.Fatalf("slim0 start: %v", err)
	}
	if err := slim1.Start(ctx); err != nil {
		t.Fatalf("slim1 start: %v", err)
	}
	if err := slim2.Start(ctx); err != nil {
		t.Fatalf("slim2 start: %v", err)
	}

	// registration reaches DB
	time.Sleep(3 * time.Second)
	require.Len(t, db.ListNodes(), 3)

	// slim-0 publishes a subscription org/test/client/0
	if err := slim0.updateSubscription(ctx, "org", "test", "client",
		0, false); err != nil {
		t.Fatalf("failed to send subcription update: %v", err)
	}

	// check that routes created in DB: from other nodes to slim-0
	waitCond(t, 3*time.Second, func() bool {
		for _, r := range db.GetRoutesForDestinationNodeID("slim-0") {
			if r.DestNodeID == "slim-0" && r.Component0 == "org" && r.Component2 == "client" {

				return true
			}
		}
		return false
	}, "wait for route for slim-0 to be created")

	// other instances should receive connections+subscriptions for slim-0
	waitCond(t, 3*time.Second, func() bool {
		_, subs1 := slim1.GetReceived()
		_, subs2 := slim2.GetReceived()
		return len(subs1) > 0 && len(subs2) > 0
	}, "wait for subs to be received by slim-1 and slim-2")

	// restart slim-1 and expect it to receive the same config
	_ = slim1.Close()
	slim1, _ = NewMockSlimServer("slim-1", 4501, target)
	if err := slim1.Start(ctx); err != nil {
		t.Fatalf("slim-1 connect: %v", err)
	}
	waitCond(t, 3*time.Second, func() bool {
		_, subs := slim1.GetReceived()
		return len(subs) > 0
	}, "wait for subscriptions to be received by slim-1")

	// restart slim-0 with different port; expect reconcilers update other nodes
	_ = slim0.Close()
	slim0, _ = NewMockSlimServer("slim-0", 4800, target)
	if err := slim0.Start(ctx); err != nil {
		t.Fatalf("slim-0 connect: %v", err)
	}

	// other instances should receive conns+subs for slim-0
	waitCond(t, 3*time.Second, func() bool {
		foundOnSlim0, foundOnSlim1 := false, false
		conns1, _ := slim1.GetReceived()
		for _, c := range conns1 {
			if c.ConnectionId == "http://127.0.0.1:4800" {
				foundOnSlim0 = true
				break
			}
		}
		conns2, _ := slim2.GetReceived()
		for _, c := range conns2 {
			if c.ConnectionId == "http://127.0.0.1:4800" {
				foundOnSlim1 = true
				break
			}
		}
		return foundOnSlim0 && foundOnSlim1
	}, "wait for changed subs to be received by slim-1 and slim-2")

	// send delete for subscription
	if err := slim0.updateSubscription(ctx, "org", "test", "client",
		0, true); err != nil {
		t.Fatalf("delete sub: %v", err)
	}

	// routes should be removed
	waitCond(t, 3*time.Second, func() bool {
		// route for slim-0 should be gone
		rs := db.GetRoutesForDestinationNodeID("slim-0")
		if len(rs) != 0 {
			return false
		}
		_, subs1 := slim1.GetReceived()
		_, subs2 := slim2.GetReceived()
		return len(subs1) == 0 && len(subs2) == 0
	}, "wait for route for slim-0 to be deleted")

	_ = slim2.Close()
	_ = slim1.Close()
	_ = slim0.Close()
}

func TestSouthbound_RouteWithConnectionError(t *testing.T) {
	db := db.NewInMemoryDBService()
	target, cleanup := startSouthbound(t, db)
	defer cleanup()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	slim0, _ := NewMockSlimServer("slim-0", 4500, target)
	slim1, _ := NewMockSlimServer("slim-1", 4501, target)
	slim1.AckConnectionError = true
	slim1.AckSubscriptionSetError = true

	if err := slim0.Start(ctx); err != nil {
		t.Fatalf("slim0 start: %v", err)
	}
	if err := slim1.Start(ctx); err != nil {
		t.Fatalf("slim1 start: %v", err)
	}

	// wait nodes in DB
	waitCond(t, 2*time.Second, func() bool { return len(db.ListNodes()) == 2 },
		"wait for 2 nodes to register")

	// slim-0 sends subscription
	if err := slim0.updateSubscription(ctx, "org", "test", "client",
		0, false); err != nil {
		t.Fatalf("send sub: %v", err)
	}

	// wait reconciler to mark routes for slim-1 as failed
	waitCond(t, 2*time.Second, func() bool {
		for _, r := range db.GetRoutesForNodeID("slim-1") {
			if r.SourceNodeID == "slim-1" && r.DestNodeID == "slim-0" && r.StatusMsg != "" {
				return true
			}
		}
		return false
	}, "wait for route slim-1:org/test/client/->slim-0 to be marked as failed")

	_ = slim0.Close()
	_ = slim1.Close()
}

func TestSouthbound_MessageHandling(t *testing.T) {
	db := db.NewInMemoryDBService()
	target, cleanup := startSouthbound(t, db)
	defer cleanup()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	// Create gRPC client
	conn, err := grpc.NewClient(target, grpc.WithTransportCredentials(insecure.NewCredentials()))
	require.NoError(t, err)
	defer conn.Close()

	client := controllerapi.NewControllerServiceClient(conn)
	stream, err := client.OpenControlChannel(ctx)
	require.NoError(t, err)

	// Receive initial ACK
	_, err = stream.Recv()
	require.NoError(t, err)

	// Send register request
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "test-register",
		Payload: &controllerapi.ControlMessage_RegisterNodeRequest{
			RegisterNodeRequest: &controllerapi.RegisterNodeRequest{
				NodeId: "slim-message-test",
				ConnectionDetails: []*controllerapi.ConnectionDetails{
					{
						Endpoint: "127.0.0.1:5010",
					},
				},
			},
		},
	})
	require.NoError(t, err)

	// Receive registration ACK
	_, err = stream.Recv()
	require.NoError(t, err)

	// Wait for and acknowledge the initial ConfigCommand from the reconciler
	configMsg, err := stream.Recv()
	require.NoError(t, err)
	if cfgCmd, ok := configMsg.Payload.(*controllerapi.ControlMessage_ConfigCommand); ok {
		// Acknowledge the config command
		ackMsg := &controllerapi.ControlMessage{
			MessageId: "ack-config",
			Payload: &controllerapi.ControlMessage_ConfigCommandAck{
				ConfigCommandAck: &controllerapi.ConfigurationCommandAck{
					OriginalMessageId: configMsg.MessageId,
				},
			},
		}
		err = stream.Send(ackMsg)
		require.NoError(t, err)
	} else {
		t.Fatalf("Expected ConfigCommand, got %T", cfgCmd)
	}

	// Test sending generic Ack (tests the Ack branch in handleNodeMessages)
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "test-ack",
		Payload: &controllerapi.ControlMessage_Ack{
			Ack: &controllerapi.Ack{
				Success: true,
			},
		},
	})
	require.NoError(t, err)

	// Give server time to process the ACK message
	time.Sleep(100 * time.Millisecond)

	// Test deregister flow
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "test-deregister",
		Payload: &controllerapi.ControlMessage_DeregisterNodeRequest{
			DeregisterNodeRequest: &controllerapi.DeregisterNodeRequest{
				Node: &controllerapi.Node{
					Id: "slim-message-test",
				},
			},
		},
	})
	require.NoError(t, err)

	// After deregister, the stream closes
	time.Sleep(100 * time.Millisecond)
	_ = stream.CloseSend()
}

// startSouthboundWithMockGroupService creates a southbound server with a mock group service
func startSouthboundWithMockGroupService(
	t *testing.T, db db.DataAccess, mockGrp *mockGroupService,
) (target string, cleanup func()) {
	ctx := util.GetContextWithLogger(context.Background(), config.LogConfig{Level: "debug"})
	cmdHandler := nodecontrol.DefaultNodeCommandHandler()
	routeService := routes.NewRouteService(db, cmdHandler,
		config.ReconcilerConfig{MaxNumOfParallelReconciles: 3, MaxRequeues: 1})
	if err := routeService.Start(ctx); err != nil {
		t.Fatalf("routeService.Start: %v", err)
	}

	s := grpc.NewServer(grpc.Creds(insecure.NewCredentials()))
	svc := NewSBAPIService(config.APIConfig{
		HTTPHost: "127.0.0.1",
		HTTPPort: "50053",
	}, config.LogConfig{Level: "debug"}, db, cmdHandler, routeService, mockGrp)
	controllerapi.RegisterControllerServiceServer(s, svc)
	l, err := net.Listen("tcp", "127.0.0.1:50053")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	go func() { _ = s.Serve(l) }()
	return l.Addr().String(), func() { s.Stop(); l.Close(); routeService.Stop() }
}

func TestSouthbound_ChannelOperations(t *testing.T) {
	db := db.NewInMemoryDBService()
	mockGrp := &mockGroupService{
		createChannelResponse: &controlplaneApi.CreateChannelResponse{
			ChannelName: "org/namespace/test-channel-123",
		},
		deleteChannelResponse: &controllerapi.Ack{
			Success: true,
		},
		listChannelsResponse: &controllerapi.ListChannelsResponse{
			ChannelName: []string{"org/namespace/channel1", "org/namespace/channel2"},
		},
	}

	target, cleanup := startSouthboundWithMockGroupService(t, db, mockGrp)
	defer cleanup()

	ctx := context.Background()

	// Create gRPC client
	conn, err := grpc.NewClient(target, grpc.WithTransportCredentials(insecure.NewCredentials()))
	require.NoError(t, err)
	defer conn.Close()

	client := controllerapi.NewControllerServiceClient(conn)
	stream, err := client.OpenControlChannel(ctx)
	require.NoError(t, err)

	// Receive initial ACK
	_, err = stream.Recv()
	require.NoError(t, err)

	// Send register request
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "test-register",
		Payload: &controllerapi.ControlMessage_RegisterNodeRequest{
			RegisterNodeRequest: &controllerapi.RegisterNodeRequest{
				NodeId: "slim-channel-test",
				ConnectionDetails: []*controllerapi.ConnectionDetails{
					{
						Endpoint: "127.0.0.1:5001",
					},
				},
			},
		},
	})
	require.NoError(t, err)

	// Receive registration ACK
	_, err = stream.Recv()
	require.NoError(t, err)

	// Wait for and acknowledge the initial ConfigCommand from the reconciler
	configMsg, err := stream.Recv()
	require.NoError(t, err)
	if _, ok := configMsg.Payload.(*controllerapi.ControlMessage_ConfigCommand); ok {
		// Acknowledge the config command
		ackMsg := &controllerapi.ControlMessage{
			MessageId: "ack-config",
			Payload: &controllerapi.ControlMessage_ConfigCommandAck{
				ConfigCommandAck: &controllerapi.ConfigurationCommandAck{
					OriginalMessageId: configMsg.MessageId,
				},
			},
		}
		err = stream.Send(ackMsg)
		require.NoError(t, err)
	}

	// Test CreateChannel
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "create-channel",
		Payload: &controllerapi.ControlMessage_CreateChannelRequest{
			CreateChannelRequest: &controllerapi.CreateChannelRequest{
				Moderators: []string{"org/namespace/identity/mod1"},
			},
		},
	})
	require.NoError(t, err)

	// Receive ACK for channel creation
	msg, err := stream.Recv()
	require.NoError(t, err)
	if ack, ok := msg.Payload.(*controllerapi.ControlMessage_Ack); ok {
		require.True(t, ack.Ack.Success)
		require.Equal(t, "create-channel", ack.Ack.OriginalMessageId)
	}

	// Test ListChannels
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "list-channels",
		Payload: &controllerapi.ControlMessage_ListChannelRequest{
			ListChannelRequest: &controllerapi.ListChannelsRequest{},
		},
	})
	require.NoError(t, err)

	// Receive list response
	msg, err = stream.Recv()
	require.NoError(t, err)
	if listResp, ok := msg.Payload.(*controllerapi.ControlMessage_ListChannelResponse); ok {
		require.Len(t, listResp.ListChannelResponse.ChannelName, 2)
		require.Contains(t, listResp.ListChannelResponse.ChannelName, "org/namespace/channel1")
	}

	// Test DeleteChannel
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "delete-channel",
		Payload: &controllerapi.ControlMessage_DeleteChannelRequest{
			DeleteChannelRequest: &controllerapi.DeleteChannelRequest{
				ChannelName: "org/namespace/test-channel",
			},
		},
	})
	require.NoError(t, err)

	// Receive ACK for channel deletion
	msg, err = stream.Recv()
	require.NoError(t, err)
	if ack, ok := msg.Payload.(*controllerapi.ControlMessage_Ack); ok {
		require.True(t, ack.Ack.Success)
		require.Equal(t, "delete-channel", ack.Ack.OriginalMessageId)
	}

	_ = stream.CloseSend()
}

func TestSouthbound_ParticipantOperations(t *testing.T) {
	db := db.NewInMemoryDBService()
	mockGrp := &mockGroupService{
		addParticipantResponse: &controllerapi.Ack{
			Success: true,
		},
		deleteParticipantResponse: &controllerapi.Ack{
			Success: true,
		},
		listParticipantsResponse: &controllerapi.ListParticipantsResponse{
			ParticipantName: []string{"org/namespace/identity/participant1", "org/namespace/identity/participant2"},
		},
	}

	target, cleanup := startSouthboundWithMockGroupService(t, db, mockGrp)
	defer cleanup()

	ctx := context.Background()

	// Create gRPC client
	conn, err := grpc.NewClient(target, grpc.WithTransportCredentials(insecure.NewCredentials()))
	require.NoError(t, err)
	defer conn.Close()

	client := controllerapi.NewControllerServiceClient(conn)
	stream, err := client.OpenControlChannel(ctx)
	require.NoError(t, err)

	// Receive initial ACK
	_, err = stream.Recv()
	require.NoError(t, err)

	// Send register request
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "test-register",
		Payload: &controllerapi.ControlMessage_RegisterNodeRequest{
			RegisterNodeRequest: &controllerapi.RegisterNodeRequest{
				NodeId: "slim-participant-test",
				ConnectionDetails: []*controllerapi.ConnectionDetails{
					{
						Endpoint: "127.0.0.1:5002",
					},
				},
			},
		},
	})
	require.NoError(t, err)

	// Receive registration ACK
	_, err = stream.Recv()
	require.NoError(t, err)

	// Wait for and acknowledge the initial ConfigCommand from the reconciler
	configMsg, err := stream.Recv()
	require.NoError(t, err)
	if _, ok := configMsg.Payload.(*controllerapi.ControlMessage_ConfigCommand); ok {
		// Acknowledge the config command
		ackMsg := &controllerapi.ControlMessage{
			MessageId: "ack-config",
			Payload: &controllerapi.ControlMessage_ConfigCommandAck{
				ConfigCommandAck: &controllerapi.ConfigurationCommandAck{
					OriginalMessageId: configMsg.MessageId,
				},
			},
		}
		err = stream.Send(ackMsg)
		require.NoError(t, err)
	}

	// Test AddParticipant
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "add-participant",
		Payload: &controllerapi.ControlMessage_AddParticipantRequest{
			AddParticipantRequest: &controllerapi.AddParticipantRequest{
				ChannelName:     "org/namespace/test-channel",
				ParticipantName: "org/namespace/identity/participant1",
			},
		},
	})
	require.NoError(t, err)

	// Receive ACK for add participant
	msg, err := stream.Recv()
	require.NoError(t, err)
	if ack, ok := msg.Payload.(*controllerapi.ControlMessage_Ack); ok {
		require.True(t, ack.Ack.Success)
		require.Equal(t, "add-participant", ack.Ack.OriginalMessageId)
	}

	// Test ListParticipants
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "list-participants",
		Payload: &controllerapi.ControlMessage_ListParticipantsRequest{
			ListParticipantsRequest: &controllerapi.ListParticipantsRequest{
				ChannelName: "org/namespace/test-channel",
			},
		},
	})
	require.NoError(t, err)

	// Receive list response
	msg, err = stream.Recv()
	require.NoError(t, err)
	if listResp, ok := msg.Payload.(*controllerapi.ControlMessage_ListParticipantsResponse); ok {
		require.Len(t, listResp.ListParticipantsResponse.ParticipantName, 2)
		require.Contains(t, listResp.ListParticipantsResponse.ParticipantName, "org/namespace/identity/participant1")
	}

	// Test DeleteParticipant
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "delete-participant",
		Payload: &controllerapi.ControlMessage_DeleteParticipantRequest{
			DeleteParticipantRequest: &controllerapi.DeleteParticipantRequest{
				ChannelName:     "org/namespace/test-channel",
				ParticipantName: "org/namespace/identity/participant1",
			},
		},
	})
	require.NoError(t, err)

	// Receive ACK for delete participant
	msg, err = stream.Recv()
	require.NoError(t, err)
	if ack, ok := msg.Payload.(*controllerapi.ControlMessage_Ack); ok {
		require.True(t, ack.Ack.Success)
		require.Equal(t, "delete-participant", ack.Ack.OriginalMessageId)
	}

	_ = stream.CloseSend()
}

func TestSouthbound_ChannelOperationsWithErrors(t *testing.T) {
	db := db.NewInMemoryDBService()
	mockGrp := &mockGroupService{
		createChannelError: fmt.Errorf("channel creation failed"),
		listChannelsError:  fmt.Errorf("listing channels failed"),
	}

	target, cleanup := startSouthboundWithMockGroupService(t, db, mockGrp)
	defer cleanup()

	ctx := context.Background()

	// Create gRPC client
	conn, err := grpc.NewClient(target, grpc.WithTransportCredentials(insecure.NewCredentials()))
	require.NoError(t, err)
	defer conn.Close()

	client := controllerapi.NewControllerServiceClient(conn)
	stream, err := client.OpenControlChannel(ctx)
	require.NoError(t, err)

	// Receive initial ACK
	_, err = stream.Recv()
	require.NoError(t, err)

	// Send register request
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "test-register",
		Payload: &controllerapi.ControlMessage_RegisterNodeRequest{
			RegisterNodeRequest: &controllerapi.RegisterNodeRequest{
				NodeId: "slim-error-test",
				ConnectionDetails: []*controllerapi.ConnectionDetails{
					{
						Endpoint: "127.0.0.1:5004",
					},
				},
			},
		},
	})
	require.NoError(t, err)

	// Receive registration ACK
	_, err = stream.Recv()
	require.NoError(t, err)

	// Acknowledge ConfigCommand
	configMsg, err := stream.Recv()
	require.NoError(t, err)
	if _, ok := configMsg.Payload.(*controllerapi.ControlMessage_ConfigCommand); ok {
		ackMsg := &controllerapi.ControlMessage{
			MessageId: "ack-config",
			Payload: &controllerapi.ControlMessage_ConfigCommandAck{
				ConfigCommandAck: &controllerapi.ConfigurationCommandAck{
					OriginalMessageId: configMsg.MessageId,
				},
			},
		}
		err = stream.Send(ackMsg)
		require.NoError(t, err)
	}

	// Test CreateChannel with error - should close stream
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "create-channel-fail",
		Payload: &controllerapi.ControlMessage_CreateChannelRequest{
			CreateChannelRequest: &controllerapi.CreateChannelRequest{
				Moderators: []string{"org/namespace/identity/mod1"},
			},
		},
	})
	require.NoError(t, err)

	// The stream should close due to the error
	time.Sleep(100 * time.Millisecond)
}

func TestSouthbound_InvalidPayload(t *testing.T) {
	db := db.NewInMemoryDBService()
	target, cleanup := startSouthbound(t, db)
	defer cleanup()

	ctx := context.Background()

	// Create gRPC client
	conn, err := grpc.NewClient(target, grpc.WithTransportCredentials(insecure.NewCredentials()))
	require.NoError(t, err)
	defer conn.Close()

	client := controllerapi.NewControllerServiceClient(conn)
	stream, err := client.OpenControlChannel(ctx)
	require.NoError(t, err)

	// Receive initial ACK
	_, err = stream.Recv()
	require.NoError(t, err)

	// Send register request
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "test-register",
		Payload: &controllerapi.ControlMessage_RegisterNodeRequest{
			RegisterNodeRequest: &controllerapi.RegisterNodeRequest{
				NodeId: "slim-invalid-test",
				ConnectionDetails: []*controllerapi.ConnectionDetails{
					{
						Endpoint: "127.0.0.1:5003",
					},
				},
			},
		},
	})
	require.NoError(t, err)

	// Receive registration ACK
	_, err = stream.Recv()
	require.NoError(t, err)

	// Wait for and acknowledge the initial ConfigCommand from the reconciler
	configMsg, err := stream.Recv()
	require.NoError(t, err)
	if _, ok := configMsg.Payload.(*controllerapi.ControlMessage_ConfigCommand); ok {
		// Acknowledge the config command
		ackMsg := &controllerapi.ControlMessage{
			MessageId: "ack-config",
			Payload: &controllerapi.ControlMessage_ConfigCommandAck{
				ConfigCommandAck: &controllerapi.ConfigurationCommandAck{
					OriginalMessageId: configMsg.MessageId,
				},
			},
		}
		err = stream.Send(ackMsg)
		require.NoError(t, err)
	}

	// Send a message with generic Ack payload to test that branch
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "test-ack",
		Payload: &controllerapi.ControlMessage_Ack{
			Ack: &controllerapi.Ack{
				Success: true,
			},
		},
	})
	require.NoError(t, err)

	// Send ConnectionListResponse to test that branch
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "test-connection-list",
		Payload: &controllerapi.ControlMessage_ConnectionListResponse{
			ConnectionListResponse: &controllerapi.ConnectionListResponse{
				Entries: []*controllerapi.ConnectionEntry{},
			},
		},
	})
	require.NoError(t, err)

	// Send SubscriptionListResponse to test that branch
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "test-subscription-list",
		Payload: &controllerapi.ControlMessage_SubscriptionListResponse{
			SubscriptionListResponse: &controllerapi.SubscriptionListResponse{
				Entries: []*controllerapi.SubscriptionEntry{},
			},
		},
	})
	require.NoError(t, err)

	// Give some time for messages to be processed
	time.Sleep(500 * time.Millisecond)

	// Stream should still be alive
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "test-final-ack",
		Payload: &controllerapi.ControlMessage_Ack{
			Ack: &controllerapi.Ack{
				Success: true,
			},
		},
	})
	require.NoError(t, err)

	_ = stream.CloseSend()
}

func TestSouthbound_NoRegistration(t *testing.T) {
	db := db.NewInMemoryDBService()
	target, cleanup := startSouthbound(t, db)
	defer cleanup()

	ctx := context.Background()

	// Create gRPC client
	conn, err := grpc.NewClient(target, grpc.WithTransportCredentials(insecure.NewCredentials()))
	require.NoError(t, err)
	defer conn.Close()

	client := controllerapi.NewControllerServiceClient(conn)
	stream, err := client.OpenControlChannel(ctx)
	require.NoError(t, err)

	// Receive initial ACK
	_, err = stream.Recv()
	require.NoError(t, err)

	// Send a non-registration message (should cause stream to close)
	err = stream.Send(&controllerapi.ControlMessage{
		MessageId: "test-invalid",
		Payload: &controllerapi.ControlMessage_Ack{
			Ack: &controllerapi.Ack{
				Success: true,
			},
		},
	})
	require.NoError(t, err)

	// Stream should close or subsequent operations should fail
	time.Sleep(500 * time.Millisecond)
}
