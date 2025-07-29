// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package nodecontroller

import (
	"github.com/spf13/cobra"

	"github.com/agntcy/slim/control-plane/common/options"
)

func NewNodeCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:     "node-connect",
		Aliases: []string{"nc"},
		Short:   "Connect directly to the node to manage connections & routes.",
		Long: `Connect directly to the node to manage connections & routes.
In this case --server should point to the node controller endpoint.`,
	}

	cmd.AddCommand(newRouteCmd(opts))
	cmd.AddCommand(newConnectionCmd(opts))

	return cmd
}
