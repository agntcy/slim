// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package controller

import (
	"context"
	"fmt"
	"io"
	"net"
	"testing"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/test/bufconn"
	"google.golang.org/protobuf/types/known/wrapperspb"

	grpcapi "github.com/agntcy/agp/control-plane/agpctl/internal/proto/controller/v1"
)

const bufSize = 1024 * 1024

var lis *bufconn.Listener

func init() {
	lis = bufconn.Listen(bufSize)

	s := grpc.NewServer()
	grpcapi.RegisterControllerServiceServer(s, &fakeServer{})

	go func() {
		if err := s.Serve(lis); err != nil {
			panic(fmt.Sprintf("bufconn server Serve failed: %v", err))
		}
	}()
}

func bufDialer(_ context.Context, _ string) (net.Conn, error) {
	return lis.Dial()
}

type fakeServer struct {
	grpcapi.UnimplementedControllerServiceServer
}

func (s *fakeServer) OpenControlChannel(
	stream grpcapi.ControllerService_OpenControlChannelServer,
) error {
	msg, err := stream.Recv()
	if err == io.EOF { //nolint:errorlint
		return nil
	}
	if err != nil {
		return err
	}

	return stream.Send(&grpcapi.ControlMessage{
		MessageId: "ack-for-" + msg.MessageId,
		Payload: &grpcapi.ControlMessage_Ack{Ack: &grpcapi.Ack{
			OriginalMessageId: msg.MessageId,
			Success:           true,
		}},
	})
}

func (s *fakeServer) ListSubscriptions(
	stream grpcapi.ControllerService_ListSubscriptionsServer,
) error {
	_, _ = stream.Recv()

	entries := []*grpcapi.SubscriptionEntry{
		{
			Company:             "org1",
			Namespace:           "ns1",
			AgentName:           "alice",
			AgentId:             &wrapperspb.UInt64Value{Value: 42},
			LocalConnectionIds:  []uint64{1},
			RemoteConnectionIds: []uint64{2},
		},
		{
			Company:             "org2",
			Namespace:           "ns2",
			AgentName:           "bob",
			AgentId:             &wrapperspb.UInt64Value{Value: 7},
			LocalConnectionIds:  []uint64{},
			RemoteConnectionIds: []uint64{3},
		},
	}
	for _, e := range entries {
		if err := stream.Send(e); err != nil {
			return err
		}
	}

	return nil
}

func TestSendConfigMessage(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	conn, err := grpc.NewClient(
		"passthrough://bufnet",
		grpc.WithContextDialer(bufDialer),
		grpc.WithTransportCredentials(insecure.NewCredentials()),
	)
	if err != nil {
		t.Fatalf("grpc.NewClient failed: %v", err)
	}
	defer conn.Close()

	client := grpcapi.NewControllerServiceClient(conn)

	stream, err := client.OpenControlChannel(ctx)
	if err != nil {
		t.Fatalf("client.OpenControlChannel failed: %v", err)
	}

	configMsg := &grpcapi.ControlMessage{
		MessageId: "test-cfg-123",
		Payload: &grpcapi.ControlMessage_ConfigCommand{
			ConfigCommand: &grpcapi.ConfigurationCommand{
				ConnectionsToCreate: []*grpcapi.Connection{{
					ConnectionId:  "c1",
					RemoteAddress: "10.0.0.1",
					RemotePort:    8080,
				}},
				RoutesToSet: []*grpcapi.Route{{
					Company:      "acme",
					Namespace:    "outshift",
					AgentName:    "agent",
					AgentId:      &wrapperspb.UInt64Value{Value: 1},
					ConnectionId: "c1",
				}},
			},
		},
	}

	if err = stream.Send(configMsg); err != nil {
		t.Fatalf("stream.Send failed: %v", err)
	}

	ack, err := stream.Recv()
	if err != nil {
		if err == io.EOF { //nolint:errorlint
			t.Fatalf("stream Recv got EOF, expected ACK message")
		}
		t.Fatalf("stream.Recv failed: %v", err)
	}

	a := ack.GetAck()
	if a == nil {
		t.Fatalf(
			"received message is not an ACK, got payload type: %T, msg: %+v",
			ack.Payload,
			ack,
		)
	}

	if a.OriginalMessageId != configMsg.MessageId {
		t.Errorf(
			"expected original_message_id '%s', got '%s'",
			configMsg.MessageId,
			a.OriginalMessageId,
		)
	}
	if !a.Success {
		t.Errorf("expected ack.Success=true, got false")
	}

	_, err = stream.Recv()
	if err != io.EOF { //nolint:errorlint
		t.Errorf(
			"expected io.EOF after receiving ACK (server should close), got err: %v",
			err,
		)
	}
}

func TestListSubscriptions(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	conn, err := grpc.NewClient(
		"passthrough://bufnet",
		grpc.WithContextDialer(bufDialer),
		grpc.WithTransportCredentials(insecure.NewCredentials()),
	)
	if err != nil {
		t.Fatalf("grpc.NewClient failed: %v", err)
	}
	defer conn.Close()

	client := grpcapi.NewControllerServiceClient(conn)

	stream, err := client.ListSubscriptions(ctx)
	if err != nil {
		t.Fatalf("client.ListSubscriptions failed: %v", err)
	}

	if err := stream.Send(&grpcapi.SubscriptionListRequest{}); err != nil {
		t.Fatalf("stream.Send failed: %v", err)
	}

	var received []*grpcapi.SubscriptionEntry
	for {
		entry, err := stream.Recv()
		if err == io.EOF { //nolint:errorlint
			break
		}
		if err != nil {
			t.Fatalf("stream.Recv failed: %v", err)
		}
		received = append(received, entry)
	}

	if len(received) != 2 {
		t.Fatalf("expected 2 entries, got %d", len(received))
	}

	e1 := received[0]
	if e1.Company != "org1" {
		t.Errorf("expected Company 'org1', got '%s'", e1.Company)
	}
	if e1.Namespace != "ns1" {
		t.Errorf("expected Namespace 'ns1', got '%s'", e1.Namespace)
	}
	if e1.AgentName != "alice" {
		t.Errorf("expected AgentName 'alice', got '%s'", e1.AgentName)
	}
	if e1.AgentId.GetValue() != 42 {
		t.Errorf("expected AgentId 42, got %d", e1.AgentId.GetValue())
	}
	if len(e1.LocalConnectionIds) != 1 || e1.LocalConnectionIds[0] != 1 {
		t.Errorf("expected LocalConnectionIds [1], got %v", e1.LocalConnectionIds)
	}
	if len(e1.RemoteConnectionIds) != 1 || e1.RemoteConnectionIds[0] != 2 {
		t.Errorf("expected RemoteConnectionIds [2], got %v", e1.RemoteConnectionIds)
	}

	e2 := received[1]
	if e2.Company != "org2" {
		t.Errorf("expected Company 'org2', got '%s'", e2.Company)
	}
	if e2.Namespace != "ns2" {
		t.Errorf("expected Namespace 'ns2', got '%s'", e2.Namespace)
	}
	if e2.AgentName != "bob" {
		t.Errorf("expected AgentName 'bob', got '%s'", e2.AgentName)
	}
	if e2.AgentId.GetValue() != 7 {
		t.Errorf("expected AgentId 7, got %d", e2.AgentId.GetValue())
	}
	if len(e2.LocalConnectionIds) != 0 {
		t.Errorf("expected LocalConnectionIds [], got %v", e2.LocalConnectionIds)
	}
	if len(e2.RemoteConnectionIds) != 1 || e2.RemoteConnectionIds[0] != 3 {
		t.Errorf("expected RemoteConnectionIds [3], got %v", e2.RemoteConnectionIds)
	}
}
