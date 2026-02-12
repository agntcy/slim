// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package controlplane

import (
	"context"
	"encoding/base64"
	"fmt"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"

	"github.com/agntcy/slim/control-plane/common/options"
	cpApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
)

func GetClient(
	ctx context.Context,
	opts *options.CommonOptions,
) (cpApi.ControlPlaneServiceClient, *grpc.ClientConn, context.Context, error) {
	var creds credentials.TransportCredentials
	if opts.TLSInsecure {
		creds = insecure.NewCredentials()
	} else if opts.TLSCAFile != "" {
		c, err := credentials.NewClientTLSFromFile(opts.TLSCAFile, "")
		if err != nil {
			return nil, nil, ctx, fmt.Errorf("loading CA file %q: %w", opts.TLSCAFile, err)
		}
		creds = c
	}

	if opts.BasicAuthCredentials != "" {
		encodedAuth := base64.StdEncoding.EncodeToString([]byte(opts.BasicAuthCredentials))
		md := metadata.New(map[string]string{
			"authorization": "Basic " + encodedAuth,
		})
		ctx = metadata.NewOutgoingContext(ctx, md)
	}

	conn, err := grpc.NewClient(
		opts.Server,
		grpc.WithTransportCredentials(creds),
	)
	if err != nil {
		return nil, nil, ctx, fmt.Errorf("error connecting to server(%s): %w", opts.Server, err)
	}

	client := cpApi.NewControlPlaneServiceClient(conn)
	return client, conn, ctx, nil
}
