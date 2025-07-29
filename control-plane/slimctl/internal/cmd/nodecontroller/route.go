// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package nodecontroller

import (
	"context"
	"fmt"
	"strings"

	"github.com/google/uuid"
	"github.com/spf13/cobra"
	"google.golang.org/protobuf/types/known/wrapperspb"

	"github.com/agntcy/slim/control-plane/common/controller"
	"github.com/agntcy/slim/control-plane/common/options"
	grpcapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	"github.com/agntcy/slim/control-plane/slimctl/internal/cmd/util"
)

func newRouteCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "route",
		Short: "Manage SLIM routes",
		Long:  `Manage SLIM routes`,
	}

	cmd.AddCommand(newListRoutesCmd(opts))
	cmd.AddCommand(newAddCmd(opts))
	cmd.AddCommand(newDelCmd(opts))

	return cmd
}

func newListRoutesCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:     "list",
		Aliases: []string{"ls"},
		Short:   "List routes",
		Long:    `List routes`,
		RunE: func(cmd *cobra.Command, _ []string) error {
			msg := &grpcapi.ControlMessage{
				MessageId: uuid.NewString(),
				Payload:   &grpcapi.ControlMessage_SubscriptionListRequest{},
			}

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			stream, err := controller.OpenControlChannel(ctx, opts)
			if err != nil {
				return fmt.Errorf("failed to open control channel: %w", err)
			}

			if err := stream.Send(msg); err != nil {
				return fmt.Errorf("failed to send control message: %w", err)
			}

			if err := stream.CloseSend(); err != nil {
				return fmt.Errorf("failed to close send: %w", err)
			}

			for {
				resp, err := stream.Recv()
				if err != nil {
					break
				}

				if listResp := resp.GetSubscriptionListResponse(); listResp != nil {
					for _, e := range listResp.Entries {
						var localNames, remoteNames []string
						for _, lc := range e.GetLocalConnections() {
							localNames = append(localNames,
								fmt.Sprintf("local:%d:%s", lc.GetId(), lc.GetConfigData()))
						}
						for _, rc := range e.GetRemoteConnections() {
							remoteNames = append(remoteNames,
								fmt.Sprintf("remote:%d:%s", rc.GetId(), rc.GetConfigData()))
						}
						fmt.Printf("%s/%s/%s id=%d local=%v remote=%v\n",
							e.GetOrganization(), e.GetNamespace(), e.GetAgentType(),
							e.GetAgentId().GetValue(),
							localNames, remoteNames,
						)
					}
				}
			}

			return nil
		},
	}
	return cmd
}

func newAddCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "add <organization/namespace/agentname/agentid> via <config_file>",
		Short: "Add a route to a SLIM instance",
		Long:  `Add a route to a SLIM instance`,
		Args:  cobra.ExactArgs(3),
		RunE: func(cmd *cobra.Command, args []string) error {
			routeID := args[0]
			viaKeyword := strings.ToLower(args[1])
			configFile := args[2]

			if viaKeyword != "via" {
				return fmt.Errorf(
					"invalid syntax: expected 'via' keyword, got '%s'",
					args[1],
				)
			}

			organization, namespace, agentType, agentID, err := util.ParseRoute(routeID)
			if err != nil {
				return err
			}

			conn, err := util.ParseConfigFile(configFile)
			if err != nil {
				return err
			}

			subscription := &grpcapi.Subscription{
				Organization: organization,
				Namespace:    namespace,
				AgentType:    agentType,
				ConnectionId: conn.ConnectionId,
				AgentId:      wrapperspb.UInt64(agentID),
			}

			msg := &grpcapi.ControlMessage{
				MessageId: uuid.NewString(),
				Payload: &grpcapi.ControlMessage_ConfigCommand{
					ConfigCommand: &grpcapi.ConfigurationCommand{
						ConnectionsToCreate:   []*grpcapi.Connection{conn},
						SubscriptionsToSet:    []*grpcapi.Subscription{subscription},
						SubscriptionsToDelete: []*grpcapi.Subscription{},
					},
				},
			}

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			stream, err := controller.OpenControlChannel(ctx, opts)
			if err != nil {
				return fmt.Errorf("failed to open control channel: %w", err)
			}

			if err = stream.Send(msg); err != nil {
				return fmt.Errorf("failed to send control message: %w", err)
			}

			if err = stream.CloseSend(); err != nil {
				return fmt.Errorf("failed to close send: %w", err)
			}

			ack, err := stream.Recv()
			if err != nil {
				return fmt.Errorf("error receiving ack via stream: %w", err)
			}

			a := ack.GetAck()
			if a == nil {
				return fmt.Errorf("unexpected response type received (not an ACK): %v", ack)
			}

			fmt.Printf(
				"ACK received for %s: success=%t\n",
				a.OriginalMessageId,
				a.Success,
			)
			if len(a.Messages) > 0 {
				for i, ackMsg := range a.Messages {
					fmt.Printf("    [%d] %s\n", i+1, ackMsg)
				}
			}

			return nil
		},
	}
	return cmd
}

func newDelCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "del <organization/namespace/agentname/agentid> via <http|https://host:port>",
		Short: "Delete a route from a SLIM instance",
		Long:  `Delete a route from a SLIM instance`,
		Args:  cobra.ExactArgs(3),
		RunE: func(cmd *cobra.Command, args []string) error {
			routeID := args[0]
			viaKeyword := strings.ToLower(args[1])
			endpoint := args[2]

			if viaKeyword != "via" {
				return fmt.Errorf(
					"invalid syntax: expected 'via' keyword, got '%s'",
					args[1],
				)
			}

			organization, namespace, agentType, agentID, err := util.ParseRoute(routeID)
			if err != nil {
				return err
			}

			_, connID, err := util.ParseEndpoint(endpoint)
			if err != nil {
				return fmt.Errorf("invalid endpoint format '%s': %w", endpoint, err)
			}

			subscription := &grpcapi.Subscription{
				Organization: organization,
				Namespace:    namespace,
				AgentType:    agentType,
				ConnectionId: connID,
				AgentId:      wrapperspb.UInt64(agentID),
			}

			msg := &grpcapi.ControlMessage{
				MessageId: uuid.NewString(),
				Payload: &grpcapi.ControlMessage_ConfigCommand{
					ConfigCommand: &grpcapi.ConfigurationCommand{
						ConnectionsToCreate:   []*grpcapi.Connection{},
						SubscriptionsToSet:    []*grpcapi.Subscription{},
						SubscriptionsToDelete: []*grpcapi.Subscription{subscription},
					},
				},
			}

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			stream, err := controller.OpenControlChannel(ctx, opts)
			if err != nil {
				return fmt.Errorf("failed to open control channel: %w", err)
			}

			if err = stream.Send(msg); err != nil {
				return fmt.Errorf("failed to send control message: %w", err)
			}

			if err = stream.CloseSend(); err != nil {
				return fmt.Errorf("failed to close send: %w", err)
			}

			ack, err := stream.Recv()
			if err != nil {
				return fmt.Errorf("error receiving ack via stream: %w", err)
			}

			a := ack.GetAck()
			if a == nil {
				return fmt.Errorf("unexpected response type received (not an ACK): %v", ack)
			}

			fmt.Printf(
				"ACK received for %s: success=%t\n",
				a.OriginalMessageId,
				a.Success,
			)
			if len(a.Messages) > 0 {
				for i, ackMsg := range a.Messages {
					fmt.Printf("    [%d] %s\n", i+1, ackMsg)
				}
			}

			return nil
		},
	}
	return cmd
}
