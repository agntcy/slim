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
	"google.golang.org/grpc/test/bufconn"
	"google.golang.org/protobuf/types/known/wrapperspb"

	"github.com/agntcy/slim/control-plane/common/options"
	grpcapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
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
				Component_0: "org1",
				Component_1: "ns1",
				Component_2: "alice",
				Id:          &wrapperspb.UInt64Value{Value: 42},
				LocalConnections: []*grpcapi.ConnectionEntry{
					{
						Id:             1,
						ConnectionType: grpcapi.ConnectionType_CONNECTION_TYPE_LOCAL,
						ConfigData:     "{\"endpoint\":\":\"}",
					},
				},
				RemoteConnections: []*grpcapi.ConnectionEntry{
					{
						Id:             2,
						ConnectionType: grpcapi.ConnectionType_CONNECTION_TYPE_REMOTE,
						ConfigData:     "{\"endpoint\":\"10.0.0.2:2500\"}",
					},
				},
			},
			{
				Component_0:      "org2",
				Component_1:      "ns2",
				Component_2:      "bob",
				Id:               &wrapperspb.UInt64Value{Value: 7},
				LocalConnections: []*grpcapi.ConnectionEntry{},
				RemoteConnections: []*grpcapi.ConnectionEntry{
					{
						Id:             3,
						ConnectionType: grpcapi.ConnectionType_CONNECTION_TYPE_REMOTE,
						ConfigData:     "{\"endpoint\":\"10.0.0.3:3500\"}",
					},
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
	case *grpcapi.ControlMessage_ConnectionListRequest:
		entries := []*grpcapi.ConnectionEntry{
			{
				Id:             1,
				ConnectionType: grpcapi.ConnectionType_CONNECTION_TYPE_LOCAL,
				ConfigData:     "{\"endpoint\":\"10.0.0.1:1000\"}",
			},
			{
				Id:             2,
				ConnectionType: grpcapi.ConnectionType_CONNECTION_TYPE_LOCAL,
				ConfigData:     "{\"endpoint\":\"10.1.1.2:2000\"}",
			},
		}
		resp := &grpcapi.ControlMessage{
			MessageId: uuid.NewString(),
			Payload: &grpcapi.ControlMessage_ConnectionListResponse{
				ConnectionListResponse: &grpcapi.ConnectionListResponse{
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

func getClientOptions() *options.CommonOptions {
	opts := options.NewOptions()
	opts.TLSInsecure = true
	opts.Server = "passthrough://bufnet"
	return opts
}

func TestSendConfigMessage(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	stream, conn, err := OpenControlChannel(ctx, getClientOptions(), grpc.WithContextDialer(bufDialer))
	if err != nil {
		t.Fatalf("OpenControlChannel failed: %v", err)
	}
	defer conn.Close()

	configMsg := &grpcapi.ControlMessage{
		MessageId: "test-cfg-123",
		Payload: &grpcapi.ControlMessage_ConfigCommand{
			ConfigCommand: &grpcapi.ConfigurationCommand{
				ConnectionsToCreate: []*grpcapi.Connection{{
					ConnectionId: "c1",
					ConfigData:   "{\"endpoint\":\"10.0.0.1:8080\"}",
				}},
				SubscriptionsToSet: []*grpcapi.Subscription{{
					Component_0:  "acme",
					Component_1:  "outshift",
					Component_2:  "agent",
					Id:           &wrapperspb.UInt64Value{Value: 1},
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

	stream, conn, err := OpenControlChannel(ctx, getClientOptions(), grpc.WithContextDialer(bufDialer))
	if err != nil {
		t.Fatalf("OpenControlChannel failed: %v", err)
	}
	defer conn.Close()

	msg := &grpcapi.ControlMessage{
		MessageId: uuid.NewString(),
		Payload:   &grpcapi.ControlMessage_SubscriptionListRequest{},
	}

	if err = stream.Send(msg); err != nil {
		t.Fatalf("stream.Send failed: %v", err)
	}
	if err := stream.CloseSend(); err != nil {
		t.Fatalf("CloseSend failed: %v", err)
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
	if e1.GetComponent_0() != "org1" ||
		e1.GetComponent_1() != "ns1" ||
		e1.GetComponent_2() != "alice" {
		t.Errorf("unexpected metadata: %+v", e1)
	}
	if e1.GetId().GetValue() != 42 {
		t.Errorf("expected Id=42, got %d", e1.GetId().GetValue())
	}
	if len(e1.LocalConnections) != 1 {
		t.Fatalf("expected 1 local connection, got %d", len(e1.LocalConnections))
	}
	lc := e1.LocalConnections[0]
	if lc.GetId() != 1 ||
		lc.GetConfigData() != "{\"endpoint\":\":\"}" {
		t.Errorf("unexpected local connection: %+v", lc)
	}

	if len(e1.RemoteConnections) != 1 {
		t.Fatalf("expected 1 remote connection, got %d", len(e1.RemoteConnections))
	}
	rc := e1.RemoteConnections[0]
	if rc.GetId() != 2 ||
		rc.GetConfigData() != "{\"endpoint\":\"10.0.0.2:2500\"}" {
		t.Errorf("unexpected remote connection: %+v", rc)
	}

	e2 := received[1]
	if e2.GetComponent_0() != "org2" || e2.GetComponent_2() != "bob" {
		t.Errorf("unexpected metadata: %+v", e2)
	}
	if len(e2.LocalConnections) != 0 {
		t.Errorf("expected no local connections, got %v", e2.LocalConnections)
	}
	if len(e2.RemoteConnections) != 1 {
		t.Fatalf("expected 1 remote connection, got %d", len(e2.RemoteConnections))
	}

	rc2 := e2.RemoteConnections[0]
	if rc2.GetId() != 3 ||
		rc2.GetConfigData() != "{\"endpoint\":\"10.0.0.3:3500\"}" {
		t.Errorf("unexpected remote connection: %+v", rc2)
	}
}

func TestListConnections(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	stream, conn, err := OpenControlChannel(ctx, getClientOptions(), grpc.WithContextDialer(bufDialer))
	if err != nil {
		t.Fatalf("OpenControlChannel failed: %v", err)
	}
	defer conn.Close()

	msg := &grpcapi.ControlMessage{
		MessageId: uuid.NewString(),
		Payload:   &grpcapi.ControlMessage_ConnectionListRequest{},
	}
	if err := stream.Send(msg); err != nil {
		t.Fatalf("Send request failed: %v", err)
	}
	if err := stream.CloseSend(); err != nil {
		t.Fatalf("CloseSend failed: %v", err)
	}

	var received []*grpcapi.ConnectionEntry
	for {
		resp, err := stream.Recv()
		if err == io.EOF { //nolint:errorlint
			break
		}
		if err != nil {
			t.Fatalf("stream.Recv failed: %v", err)
		}

		if listResp := resp.GetConnectionListResponse(); listResp != nil {
			received = append(received, listResp.Entries...)
		}
	}

	if len(received) != 2 {
		t.Fatalf("expected 2 entries, got %d", len(received))
	}
	if received[0].GetId() != 1 || received[1].GetId() != 2 {
		t.Errorf("unexpected entries: %+v", received)
	}
}
