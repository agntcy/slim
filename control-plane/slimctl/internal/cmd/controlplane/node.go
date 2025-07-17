// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package controlplane

import (
	"context"
	"fmt"

	cpApi "github.com/agntcy/slim/control-plane/common/controlplane"
	"github.com/agntcy/slim/control-plane/common/options"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/spf13/cobra"
)

func newNodeCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "node",
		Short: "Manage SLIM nodes",
		Long:  `Manage SLIM nodes`,
	}

	cmd.AddCommand(newListNodesCmd(opts))

	return cmd
}

func newListNodesCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "list",
		Short: "List nodes",
		Long:  `List nodes connected to the control plane`,
		RunE: func(cmd *cobra.Command, _ []string) error {

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpCLient, err := cpApi.GetClient(ctx, opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}
			listResponse, err := cpCLient.ListNodes(ctx, &controlplaneApi.NodeListRequest{})
			if err != nil {
				return fmt.Errorf("failed to list nodes: %w", err)
			}

			// iterate through the nodes and print their details
			for _, node := range listResponse.Entries {
				fmt.Printf("Node ID: %s, Host: %s, Port: %s\n",
					node.Id, node.Host, node.Port)
			}

			return nil
		},
	}
	return cmd
}
