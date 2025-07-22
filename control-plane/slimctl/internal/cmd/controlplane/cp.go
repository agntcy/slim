// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package controlplane

import (
	"github.com/spf13/cobra"

	"github.com/agntcy/slim/control-plane/common/options"
)

func NewCpCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "cp",
		Short: "Manage SLIM nodes through the control plane",
		Long:  `Manage SLIM node routes etc. through the control plane`,
	}

	cmd.AddCommand(newNodeCmd(opts))
	cmd.AddCommand(newRouteCmd(opts))
	cmd.AddCommand(newConnectionCmd(opts))

	return cmd
}
