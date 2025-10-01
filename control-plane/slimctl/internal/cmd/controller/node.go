// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package controller

import (
	"context"
	"fmt"

	"github.com/spf13/cobra"

	cpApi "github.com/agntcy/slim/control-plane/common/controlplane"
	"github.com/agntcy/slim/control-plane/common/options"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
)

func NewNodeCmd(opts *options.CommonOptions) *cobra.Command {
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
		Use:     "list",
		Aliases: []string{"ls"},
		Short:   "List nodes",
		Long:    `List nodes connected to the control plane`,
		RunE: func(cmd *cobra.Command, _ []string) error {
			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpClient, ctx, err := cpApi.GetClient(ctx, opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}
			listResponse, err := cpClient.ListNodes(ctx, &controlplaneApi.NodeListRequest{})
			if err != nil {
				return fmt.Errorf("failed to list nodes: %w", err)
			}

			// iterate through the nodes and print their details
			for _, node := range listResponse.Entries {
				fmt.Printf("Node ID: %s status: %s\n", node.Id, node.Status)
				// print connection details if available
				if len(node.Connections) > 0 {
					fmt.Println("  Connection details:")
					for _, conn := range node.Connections {
						fmt.Printf("  - Endpoint: %s\n", conn.Endpoint)
						fmt.Printf("    MtlsRequired: %v\n", conn.MtlsRequired)
						if conn.ExternalEndpoint != nil {
							fmt.Printf("    ExternalEndpoint: %s\n", *conn.ExternalEndpoint)
						}
						if conn.GroupName != nil {
							fmt.Printf("    GroupName: %s\n", *conn.GroupName)
						}
					}
				} else {
					fmt.Println("No connection details available")
				}
			}

			return nil
		},
	}
	return cmd
}
