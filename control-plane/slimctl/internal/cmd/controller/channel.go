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
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
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

			cpCLient, err := cpApi.GetClient(opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}

			createChannelResponse, err := cpCLient.CreateChannel(ctx, &controlplaneApi.CreateChannelRequest{
				Moderators: moderators,
			})
			if err != nil {
				return fmt.Errorf("failed to create channel: %w", err)
			}
			channelID := createChannelResponse.GetChannelId()

			fmt.Printf("Received response: %v\n", channelID)
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
			channelID := args[0]
			if channelID == "" {
				return fmt.Errorf("channelID cannot be empty")
			}

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpCLient, err := cpApi.GetClient(opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}

			deleteChannelResponse, err := cpCLient.DeleteChannel(ctx, &grpcapi.DeleteChannelRequest{
				ChannelId: channelID,
			})
			if err != nil {
				return fmt.Errorf("failed to delete channel: %w", err)
			}
			if !deleteChannelResponse.Success {
				return fmt.Errorf("failed to delete channel")
			}
			fmt.Printf("Channel deleted successfully with ID: %v\n", channelID)
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

			cpCLient, err := cpApi.GetClient(opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}

			listChannelsResponse, err := cpCLient.ListChannels(ctx, &grpcapi.ListChannelsRequest{})
			if err != nil {
				return fmt.Errorf("failed to list channels: %w", err)
			}

			channelIDs := listChannelsResponse.GetChannelId()
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
	return moderators, nil
}
