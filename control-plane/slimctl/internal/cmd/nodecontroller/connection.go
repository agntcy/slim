// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package nodecontroller

import (
	"context"
	"fmt"

	"github.com/google/uuid"
	"github.com/spf13/cobra"

	"github.com/agntcy/slim/control-plane/common/controller"
	"github.com/agntcy/slim/control-plane/common/options"
	grpcapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
)

func newConnectionCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:     "connection",
		Aliases: []string{"conn"},
		Short:   "Manage SLIM connections",
		Long:    `Manage SLIM connections`,
	}

	cmd.AddCommand(newListCmd(opts))

	return cmd
}

func newListCmd(opts *options.CommonOptions) *cobra.Command {
	return &cobra.Command{
		Use:     "list",
		Aliases: []string{"ls"},
		Short:   "List active connections",
		Long:    `List active connections`,
		RunE: func(cmd *cobra.Command, _ []string) error {
			msg := &grpcapi.ControlMessage{
				MessageId: uuid.NewString(),
				Payload:   &grpcapi.ControlMessage_ConnectionListRequest{},
			}

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			stream, err := controller.OpenControlChannel(ctx, opts)
			if err != nil {
				return fmt.Errorf("open control channel: %w", err)
			}

			if err := stream.Send(msg); err != nil {
				return fmt.Errorf("send request: %w", err)
			}
			if err := stream.CloseSend(); err != nil {
				return fmt.Errorf("close send: %w", err)
			}

			for {
				resp, err := stream.Recv()
				if err != nil {
					break
				}

				if listResp := resp.GetConnectionListResponse(); listResp != nil {
					for _, e := range listResp.Entries {
						fmt.Printf("id=%d %s\n",
							e.GetId(),
							e.GetConfigData(),
						)
					}
				}
			}

			return nil
		},
	}
}
