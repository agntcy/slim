// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package controller

import (
	"context"
	"fmt"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/credentials/insecure"

	"github.com/agntcy/agp/control-plane/agpctl/internal/options"
	grpcapi "github.com/agntcy/agp/control-plane/agpctl/internal/proto/controller/v1"
)

func SendConfigMessage(
	ctx context.Context,
	opts *options.CommonOptions,
	msg *grpcapi.ControlMessage,
) (*grpcapi.ControlMessage, error) {
	opCtx, cancel := context.WithTimeout(ctx, opts.Timeout)
	defer cancel()

	var creds credentials.TransportCredentials
	if opts.TLSInsecure {
		creds = insecure.NewCredentials()
	} else if opts.TLSCAFile != "" {
		c, err := credentials.NewClientTLSFromFile(opts.TLSCAFile, "")
		if err != nil {
			return nil, fmt.Errorf("loading CA file %q: %w", opts.TLSCAFile, err)
		}
		creds = c
	}

	conn, err := grpc.NewClient(
		opts.Server,
		grpc.WithTransportCredentials(creds),
	)
	if err != nil {
		return nil, fmt.Errorf("grpc.NewClient(%s) failed: %w", opts.Server, err)
	}
	defer conn.Close()

	client := grpcapi.NewControllerServiceClient(conn)

	stream, err := client.OpenControlChannel(opCtx)
	if err != nil {
		return nil, fmt.Errorf(
			"cannot open control channel to %s: %w",
			opts.Server,
			err,
		)
	}

	if err = stream.Send(msg); err != nil {
		return nil, fmt.Errorf(
			"cannot send config message via stream: %w",
			err,
		)
	}

	if err = stream.CloseSend(); err != nil {
		return nil, fmt.Errorf(
			"cannot send Close via stream: %w",
			err,
		)
	}

	ack, err := stream.Recv()
	if err != nil {
		return nil, fmt.Errorf("error receiving ack via stream: %w", err)
	}

	return ack, nil
}
