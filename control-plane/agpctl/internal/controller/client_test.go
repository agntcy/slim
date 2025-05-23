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

	"github.com/google/uuid"
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
	if err != nil {
		return err
	}
	switch msg.Payload.(type) {
	case *grpcapi.ControlMessage_ConfigCommand:
		reply := &grpcapi.ControlMessage{
			MessageId: "ack-for-" + msg.MessageId,
			Payload: &grpcapi.ControlMessage_Ack{Ack: &grpcapi.Ack{
				OriginalMessageId: msg.MessageId,
				Success:           true,
			}},
		}
		if err := stream.Send(reply); err != nil {
			return err
		}
	case *grpcapi.ControlMessage_SubscriptionListRequest:
		entries := []*grpcapi.SubscriptionEntry{
			{
				Company:   "org1",
				Namespace: "ns1",
				AgentName: "alice",
				AgentId:   &wrapperspb.UInt64Value{Value: 42},
				LocalConnections: []*grpcapi.ConnectionEntry{
					{Id: 1, Name: "local:1"},
				},
				RemoteConnections: []*grpcapi.ConnectionEntry{
					{Id: 2, Name: "remote:unknown:unknown:2"},
				},
			},
			{
				Company:          "org2",
				Namespace:        "ns2",
				AgentName:        "bob",
				AgentId:          &wrapperspb.UInt64Value{Value: 7},
				LocalConnections: []*grpcapi.ConnectionEntry{},
				RemoteConnections: []*grpcapi.ConnectionEntry{
					{Id: 3, Name: "remote:unknown:unknown:3"},
				},
			},
		}
		resp := &grpcapi.ControlMessage{
			MessageId: uuid.NewString(),
			Payload: &grpcapi.ControlMessage_SubscriptionListResponse{
				SubscriptionListResponse: &grpcapi.SubscriptionListResponse{
					Entries: entries,
				},
			},
		}
		if err := stream.Send(resp); err != nil {
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

	stream, err := client.OpenControlChannel(ctx)
	if err != nil {
		t.Fatalf("client.OpenControlChannel failed: %v", err)
	}

	msg := &grpcapi.ControlMessage{
		MessageId: uuid.NewString(),
		Payload:   &grpcapi.ControlMessage_SubscriptionListRequest{},
	}

	if err = stream.Send(msg); err != nil {
		t.Fatalf("stream.Send failed: %v", err)
	}

	var received []*grpcapi.SubscriptionEntry
	for {
		resp, err := stream.Recv()
		if err == io.EOF { //nolint:errorlint
			break
		}
		if err != nil {
			t.Fatalf("stream.Recv failed: %v", err)
		}

		if listResp := resp.GetSubscriptionListResponse(); listResp != nil {
			received = append(received, listResp.Entries...)
		}
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
	if len(e1.LocalConnections) != 1 {
		t.Fatalf("expected 1 local connection, got %d", len(e1.LocalConnections))
	}
	lc := e1.LocalConnections[0]
	if lc.Id != 1 || lc.Name != "local:1" {
		t.Errorf("expected local connection {Id:1,Name:local:1}, got %+v", lc)
	}
	if len(e1.RemoteConnections) != 1 {
		t.Fatalf("expected 1 remote connection, got %d", len(e1.RemoteConnections))
	}
	rc := e1.RemoteConnections[0]
	if rc.Id != 2 || rc.Name != "remote:unknown:unknown:2" {
		t.Errorf(
			"expected remote connection {Id:2,Name:remote:unknown:unknown:2}, got %+v",
			rc,
		)
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
	if len(e2.LocalConnections) != 0 {
		t.Errorf("expected no local connections, got %v", e2.LocalConnections)
	}
	if len(e2.RemoteConnections) != 1 {
		t.Fatalf("expected 1 remote connection, got %d", len(e2.RemoteConnections))
	}
	rc2 := e2.RemoteConnections[0]
	if rc2.Id != 3 || rc2.Name != "remote:unknown:unknown:3" {
		t.Errorf(
			"expected remote connection {Id:3,Name:remote:unknown:unknown:3}, got %+v",
			rc2,
		)
	}
}
