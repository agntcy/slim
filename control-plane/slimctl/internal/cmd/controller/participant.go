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
		Short: "delete participantID",
		Long:  "delete participantID",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			channelID, _ := cmd.Flags().GetString(channelIDFlag)
			if channelID == "" {
				return fmt.Errorf("channel ID is required")
			}
			participantID := args[0]
			if participantID == "" {
				return fmt.Errorf("participant ID is required")
			}
			fmt.Printf("Deleting participant from channel ID %s: %v\n", channelID, participantID)

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpCLient, err := cpApi.GetClient(opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}

			deleteParticipantResponse, err := cpCLient.DeleteParticipant(ctx, &grpcapi.DeleteParticipantRequest{
				ParticipantId: participantID,
				ChannelId:     channelID,
			})
			if err != nil {
				return fmt.Errorf("failed to delete participant: %w", err)
			}
			if !deleteParticipantResponse.Success {
				return fmt.Errorf("failed to delete participant")
			}
			fmt.Printf("Participant deleted successfully from channel ID %s: %v\n", channelID, participantID)
			return nil
		},
	}
	return cmd
}

func newAddParticipantCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "add",
		Short: "add participantID",
		Long:  "add participantID",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			channelID, _ := cmd.Flags().GetString(channelIDFlag)
			if channelID == "" {
				return fmt.Errorf("channel ID is required")
			}
			participantID := args[0]
			if participantID == "" {
				return fmt.Errorf("participant ID is required")
			}

			fmt.Printf("Adding participant to channel ID %s: %v\n", channelID, participantID)

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpCLient, err := cpApi.GetClient(opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}

			addParticipantResponse, err := cpCLient.AddParticipant(ctx, &grpcapi.AddParticipantRequest{
				ParticipantId: participantID,
				ChannelId:     channelID,
			})
			if err != nil {
				return fmt.Errorf("failed to add participants: %w", err)
			}
			if !addParticipantResponse.Success {
				return fmt.Errorf("failed to add participants")
			}
			fmt.Printf("Participant added successfully to channel ID %s: %v\n", channelID, participantID)
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
			channelID, _ := cmd.Flags().GetString(channelIDFlag)
			if channelID == "" {
				return fmt.Errorf("channel ID is required")
			}
			fmt.Printf("Listing participants for channel ID: %s\n", channelID)

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpCLient, err := cpApi.GetClient(opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}

			listParticipantsResponse, err := cpCLient.ListParticipants(ctx, &grpcapi.ListParticipantsRequest{
				ChannelId: channelID,
			})
			if err != nil {
				return fmt.Errorf("failed to list participants: %w", err)
			}
			participantIDs := listParticipantsResponse.GetParticipantId()
			fmt.Printf("Following participants found for channel ID %s: %v\n", channelID, participantIDs)
			return nil
		},
	}
	return cmd
}
