// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package controller

import (
	"context"
	"fmt"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"

	grpcapi "github.com/agntcy/agp/control-plane/internal/proto/controller/v1"
)

func SendConfigMessage(
	ctx context.Context,
	serverAddr string,
	msg *grpcapi.ControlMessage,
) (*grpcapi.ControlMessage, error) {
	// TODO(zkacsand): make the timeout configurable
	opCtx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()

	conn, err := grpc.NewClient(
		serverAddr,
		grpc.WithTransportCredentials(insecure.NewCredentials()),
	)
	if err != nil {
		return nil, fmt.Errorf("grpc.NewClient(%s) failed: %w", serverAddr, err)
	}
	defer conn.Close()

	client := grpcapi.NewControllerServiceClient(conn)

	stream, err := client.OpenControlChannel(opCtx)
	if err != nil {
		return nil, fmt.Errorf(
			"cannot open control channel to %s: %w",
			serverAddr,
			err,
		)
	}

	if err = stream.Send(msg); err != nil {
		return nil, fmt.Errorf(
			"cannot send config message via stream: %w",
			err,
		)
	}

	ack, err := stream.Recv()
	if err != nil {
		return nil, fmt.Errorf("error receiving ack via stream: %w", err)
	}

	return ack, nil
}
