// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package controlplane

import (
	"fmt"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/credentials/insecure"

	"github.com/agntcy/slim/control-plane/common/options"
	cpApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
)

func GetClient(
	opts *options.CommonOptions,
) (cpApi.ControlPlaneServiceClient, error) {
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

	client := cpApi.NewControlPlaneServiceClient(conn)
	return client, nil
}
