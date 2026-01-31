// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package controller

import (
	"context"
	"encoding/base64"
	"fmt"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"

	"github.com/agntcy/slim/control-plane/common/options"
	controllerApi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
)

func OpenControlChannel(
	ctx context.Context,
	opts *options.CommonOptions,
	dialOpts ...grpc.DialOption,
) (controllerApi.ControllerService_OpenControlChannelClient, *grpc.ClientConn, error) {
	var creds credentials.TransportCredentials
	if opts.TLSInsecure {
		creds = insecure.NewCredentials()
	} else if opts.TLSCAFile != "" {
		c, err := credentials.NewClientTLSFromFile(opts.TLSCAFile, "")
		if err != nil {
			return nil, nil, fmt.Errorf("loading CA file %q: %w", opts.TLSCAFile, err)
		}
		creds = c
	}

	allOpts := append([]grpc.DialOption{grpc.WithTransportCredentials(creds)}, dialOpts...)

	conn, err := grpc.NewClient(
		opts.Server,
		allOpts...,
	)
	if err != nil {
		return nil, nil, fmt.Errorf("error connecting to server(%s): %w", opts.Server, err)
	}

	if opts.BasicAuthCredentials != "" {
		encodedAuth := base64.StdEncoding.EncodeToString([]byte(opts.BasicAuthCredentials))
		md := metadata.New(map[string]string{
			"authorization": "Basic " + encodedAuth,
		})
		ctx = metadata.NewOutgoingContext(ctx, md)
	}

	client := controllerApi.NewControllerServiceClient(conn)
	stream, err := client.OpenControlChannel(ctx)
	if err != nil {
		conn.Close()
		return nil, nil, fmt.Errorf(
			"cannot open control channel to %s: %w",
			opts.Server,
			err,
		)
	}

	return stream, conn, nil
}
