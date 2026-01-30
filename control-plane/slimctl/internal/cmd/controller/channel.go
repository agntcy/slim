// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package controller

import (
	"context"
	"fmt"
	"strings"

	"github.com/spf13/cobra"

	cpApi "github.com/agntcy/slim/control-plane/common/controlplane"
	"github.com/agntcy/slim/control-plane/common/options"
	grpcapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplanev1 "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/common/util"
)

func NewChannelCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "channel",
		Short: "Manage SLIM channels",
		Long:  `Manage SLIM channels`,
	}

	cmd.AddCommand(newCreateChannelCmd(opts))
	cmd.AddCommand(newDeleteChannelCmd(opts))
	cmd.AddCommand(newListChannelsCmd(opts))

	return cmd
}

func newCreateChannelCmd(opts *options.CommonOptions) *cobra.Command {
	return &cobra.Command{
		Use:   "create moderators=moderator1,moderator2",
		Short: "Create new channel",
		Long:  "Create new channel",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			moderatorsParam := args[0]
			moderators, err := getModerators(moderatorsParam)
			if err != nil {
				return fmt.Errorf("failed to parse moderators: %w", err)
			}

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpClient, conn, ctx, err := cpApi.GetClient(ctx, opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}
			defer conn.Close()

			createChannelResponse, err := cpClient.CreateChannel(ctx, &controlplanev1.CreateChannelRequest{
				Moderators: moderators,
			})
			if err != nil {
				return fmt.Errorf("failed to create channel: %w", err)
			}
			channelName := createChannelResponse.GetChannelName()

			fmt.Printf("Received response: %v\n", channelName)
			return nil
		},
	}
}

func newDeleteChannelCmd(opts *options.CommonOptions) *cobra.Command {
	return &cobra.Command{
		Use:   "delete channelID",
		Short: "Delete a channel",
		Long:  "Delete a channel",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			channelName := args[0]

			// validate channel name format
			if _, err := util.ValidateName(channelName, 3); err != nil {
				return err
			}

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpClient, conn, ctx, err := cpApi.GetClient(ctx, opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}
			defer conn.Close()

			deleteChannelResponse, err := cpClient.DeleteChannel(ctx, &grpcapi.DeleteChannelRequest{
				ChannelName: channelName,
			})
			if err != nil {
				return fmt.Errorf("failed to delete channel: %w", err)
			}
			if !deleteChannelResponse.Success {
				return fmt.Errorf("failed to delete channel: unsuccessful response")
			}
			fmt.Printf("Channel deleted successfully with ID: %v\n", channelName)
			return nil
		},
	}
}

func newListChannelsCmd(opts *options.CommonOptions) *cobra.Command {
	return &cobra.Command{
		Use:   "list",
		Short: "List channels",
		Long:  "List channels",
		RunE: func(cmd *cobra.Command, _ []string) error {
			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpClient, conn, ctx, err := cpApi.GetClient(ctx, opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}
			defer conn.Close()

			listChannelsResponse, err := cpClient.ListChannels(ctx, &grpcapi.ListChannelsRequest{})
			if err != nil {
				return fmt.Errorf("failed to list channels: %w", err)
			}

			channelIDs := listChannelsResponse.GetChannelName()
			fmt.Printf("Following channels found: %v\n", channelIDs)
			return nil
		},
	}
}

func getModerators(param string) ([]string, error) {
	parts := strings.Split(param, "=")
	if len(parts) != 2 {
		return nil, fmt.Errorf("invalid syntax: expected 'moderators=moderator1,moderator2', got '%s'", param)
	}
	if parts[0] != "moderators" {
		return nil, fmt.Errorf("invalid syntax: expected keyword 'moderators', got '%s'", parts[0])
	}

	moderators := strings.Split(parts[1], ",")
	if len(moderators) == 0 {
		return nil, fmt.Errorf("no moderators specified")
	}

	// Validate moderator names
	for _, mod := range moderators {
		if _, err := util.ValidateName(mod, 4); err != nil {
			return nil, fmt.Errorf("invalid moderator name '%s': %w", mod, err)
		}
	}

	return moderators, nil
}
