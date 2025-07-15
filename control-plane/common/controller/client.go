// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package controller

import (
	"context"
	"fmt"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/credentials/insecure"

	"github.com/agntcy/slim/control-plane/common/options"
	controllerApi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
)

func OpenControlChannel(
	ctx context.Context,
	opts *options.CommonOptions,
) (controllerApi.ControllerService_OpenControlChannelClient, error) {
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
		return nil, fmt.Errorf("error connecting to server(%s): %w", opts.Server, err)
	}

	client := controllerApi.NewControllerServiceClient(conn)
	stream, err := client.OpenControlChannel(ctx)
	if err != nil {
		conn.Close()
		return nil, fmt.Errorf(
			"cannot open control channel to %s: %w",
			opts.Server,
			err,
		)
	}

	return stream, nil
}
