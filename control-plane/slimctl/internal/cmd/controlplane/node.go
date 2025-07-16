// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package controlplane

import (
	"context"

	"github.com/agntcy/slim/control-plane/slimctl/internal/options"
	"github.com/spf13/cobra"
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
		Use:   "list",
		Short: "List nodes",
		Long:  `List nodes connected to the control plane`,
		RunE: func(cmd *cobra.Command, _ []string) error {

			_, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()
			return nil
		},
	}
	return cmd
}
