// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package controller

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"testing"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/test/bufconn"
	"google.golang.org/protobuf/types/known/wrapperspb"

	grpcapi "github.com/agntcy/agp/control-plane/internal/proto/controller/v1"
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
	if errors.Is(err, io.EOF) {
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
		if errors.Is(err, io.EOF) {
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
	if errors.Is(err, io.EOF) {
		t.Errorf(
			"expected io.EOF after receiving ACK (server should close), got err: %v",
			err,
		)
	}
}
