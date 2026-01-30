// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package controller

import (
	"context"
	"fmt"

	"github.com/spf13/cobra"

	cpApi "github.com/agntcy/slim/control-plane/common/controlplane"
	"github.com/agntcy/slim/control-plane/common/options"
	grpcapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
)

const channelIDFlag = "channel-id"

func NewParticipantCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "participant",
		Short: "Manage channel participants",
		Long:  `Manage channel participants`,
	}

	cmd.PersistentFlags().StringP(channelIDFlag, "c", "", "ID of the channel to manage participants for")
	err := cmd.MarkPersistentFlagRequired(channelIDFlag)
	if err != nil {
		fmt.Printf("Error marking persistent flag required: %v\n", err)
	}

	cmd.AddCommand(newAddParticipantCmd(opts))
	cmd.AddCommand(newDeleteParticipantCmd(opts))
	cmd.AddCommand(newListParticipantsCmd(opts))

	return cmd
}

func newDeleteParticipantCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "delete",
		Short: "delete ParticipantName",
		Long:  "delete ParticipantName",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			channelID, _ := cmd.Flags().GetString(channelIDFlag)
			if channelID == "" {
				return fmt.Errorf("channel ID is required")
			}
			ParticipantName := args[0]
			if ParticipantName == "" {
				return fmt.Errorf("participant ID is required")
			}
			fmt.Printf("Deleting participant from channel ID %s: %v\n", channelID, ParticipantName)

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpClient, conn, ctx, err := cpApi.GetClient(ctx, opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}
			defer conn.Close()

			deleteParticipantResponse, err := cpClient.DeleteParticipant(ctx, &grpcapi.DeleteParticipantRequest{
				ParticipantName: ParticipantName,
				ChannelName:     channelID,
			})
			if err != nil {
				return fmt.Errorf("failed to delete participant: %w", err)
			}
			if !deleteParticipantResponse.Success {
				return fmt.Errorf("failed to delete participant: unsuccessful response")
			}
			fmt.Printf("Participant deleted successfully from channel ID %s: %v\n", channelID, ParticipantName)
			return nil
		},
	}
	return cmd
}

func newAddParticipantCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "add",
		Short: "add ParticipantName",
		Long:  "add ParticipantName",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			channelID, _ := cmd.Flags().GetString(channelIDFlag)
			if channelID == "" {
				return fmt.Errorf("channel ID is required")
			}
			ParticipantName := args[0]
			if ParticipantName == "" {
				return fmt.Errorf("participant ID is required")
			}

			fmt.Printf("Adding participant to channel ID %s: %v\n", channelID, ParticipantName)

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpClient, conn, ctx, err := cpApi.GetClient(ctx, opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}
			defer conn.Close()

			addParticipantResponse, err := cpClient.AddParticipant(ctx, &grpcapi.AddParticipantRequest{
				ParticipantName: ParticipantName,
				ChannelName:     channelID,
			})
			if err != nil {
				return fmt.Errorf("failed to add participants: %w", err)
			}
			if !addParticipantResponse.Success {
				return fmt.Errorf("failed to add participants: unsuccessful response")
			}
			fmt.Printf("Participant added successfully to channel ID %s: %v\n", channelID, ParticipantName)
			return nil
		},
	}
	return cmd
}

func newListParticipantsCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "list",
		Short: "List participants",
		Long:  `List participants`,
		RunE: func(cmd *cobra.Command, _ []string) error {
			channelName, _ := cmd.Flags().GetString(channelIDFlag)
			if channelName == "" {
				return fmt.Errorf("channel ID is required")
			}

			fmt.Printf("Listing participants for channel ID: %s\n", channelName)
			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpClient, conn, ctx, err := cpApi.GetClient(ctx, opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}
			defer conn.Close()

			listParticipantsResponse, err := cpClient.ListParticipants(ctx, &grpcapi.ListParticipantsRequest{
				ChannelName: channelName,
			})
			if err != nil {
				return fmt.Errorf("failed to list participants: %w", err)
			}
			ParticipantNames := listParticipantsResponse.GetParticipantName()
			fmt.Printf("Following participants found for channel ID %s: %v\n", channelName, ParticipantNames)
			return nil
		},
	}
	return cmd
}
