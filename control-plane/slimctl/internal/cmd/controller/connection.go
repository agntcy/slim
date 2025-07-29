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

func NewConnectionCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:     "connection",
		Aliases: []string{"conn"},
		Short:   "Manage SLIM connections",
		Long:    `Manage SLIM connections`,
	}

	cmd.PersistentFlags().StringP(nodeIDFlag, "n", "", "ID of the node to manage routes for")
	err := cmd.MarkPersistentFlagRequired(nodeIDFlag)
	if err != nil {
		fmt.Printf("Error marking persistent flag required: %v\n", err)
	}

	cmd.AddCommand(newListConnectionsCmd(opts))

	return cmd
}

func newListConnectionsCmd(opts *options.CommonOptions) *cobra.Command {
	return &cobra.Command{
		Use:     "list",
		Aliases: []string{"ls"},
		Short:   "List active connections",
		Long:    `List active connections`,
		RunE: func(cmd *cobra.Command, _ []string) error {
			nodeID, _ := cmd.Flags().GetString(nodeIDFlag)
			fmt.Printf("Listing connections for node ID: %s\n", nodeID)

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpCLient, err := cpApi.GetClient(opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}

			connectionListResponse, err := cpCLient.ListConnections(ctx, &controlplaneApi.Node{
				Id: nodeID,
			})
			if err != nil {
				return fmt.Errorf("failed to list connections: %w", err)
			}
			fmt.Printf("Received connection list response: %v\n", len(connectionListResponse.Entries))
			for _, entry := range connectionListResponse.Entries {
				fmt.Printf(
					"Connection ID: %v, Connection type: %v, ConfigData %v\n",
					entry.Id,
					entry.ConnectionType,
					entry.ConfigData,
				)
			}

			return nil
		},
	}
}
