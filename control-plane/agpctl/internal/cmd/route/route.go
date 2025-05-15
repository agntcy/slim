// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package route

import (
	"context"
	"fmt"
	"net"
	"strconv"
	"strings"

	"github.com/google/uuid"
	"github.com/spf13/cobra"
	"google.golang.org/protobuf/types/known/wrapperspb"

	"github.com/agntcy/agp/control-plane/agpctl/internal/controller"
	"github.com/agntcy/agp/control-plane/agpctl/internal/options"
	grpcapi "github.com/agntcy/agp/control-plane/agpctl/internal/proto/controller/v1"
)

func NewRouteCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "route",
		Short: "Manage gateway routes",
		Long:  `Manage gateway routes`,
	}

	cmd.AddCommand(newListCmd(opts))
	cmd.AddCommand(newAddCmd(opts))
	cmd.AddCommand(newDelCmd(opts))

	return cmd
}

func newListCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "list",
		Short: "List routes",
		Long:  `List routes`,
		RunE: func(cmd *cobra.Command, _ []string) error {
			stream, err := controller.ListSubscriptions(cmd.Context(), opts)
			if err != nil {
				return fmt.Errorf("failed to list subscriptions: %w", err)
			}

			if err := stream.Send(&grpcapi.SubscriptionListRequest{}); err != nil {
				return fmt.Errorf("failed to send list request: %w", err)
			}

			if err := stream.CloseSend(); err != nil {
				return fmt.Errorf("close send failed: %w", err)
			}

			for {
				entry, err := stream.Recv()
				if err != nil {
					break
				}
				fmt.Printf("%s/%s/%s id=%d local=%v remote=%v\n",
					entry.Company,
					entry.Namespace,
					entry.AgentName,
					entry.AgentId.GetValue(),
					entry.LocalConnectionIds,
					entry.RemoteConnectionIds,
				)
			}

			return nil
		},
	}
	return cmd
}

func newAddCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "add <organization/namespace/agentname/agentid> via <host:port>",
		Short: "Add a route to the gateway",
		Long:  `Add a route to the gateway`,
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

			company, namespace, agentName, agentID, err := parseRoute(routeID)
			if err != nil {
				return err
			}

			conn, connID, err := parseEndpoint(endpoint)
			if err != nil {
				return err
			}

			route := &grpcapi.Route{
				Company:      company,
				Namespace:    namespace,
				AgentName:    agentName,
				ConnectionId: connID,
				AgentId:      wrapperspb.UInt64(agentID),
			}

			msg := &grpcapi.ControlMessage{
				MessageId: uuid.NewString(),
				Payload: &grpcapi.ControlMessage_ConfigCommand{
					ConfigCommand: &grpcapi.ConfigurationCommand{
						ConnectionsToCreate: []*grpcapi.Connection{conn},
						RoutesToSet:         []*grpcapi.Route{route},
						RoutesToDelete:      []*grpcapi.Route{},
					},
				},
			}

			return sendConfigMessage(cmd.Context(), opts, msg)
		},
	}
	return cmd
}

func newDelCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "del <organization/namespace/agentname/agentid> via <host:port>",
		Short: "Delete a route from the gateway",
		Long:  `Delete a route from the gateway`,
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

			company, namespace, agentName, agentID, err := parseRoute(routeID)
			if err != nil {
				return err
			}

			_, connID, err := parseEndpoint(endpoint)
			if err != nil {
				return fmt.Errorf("invalid endpoint format '%s': %w", endpoint, err)
			}

			route := &grpcapi.Route{
				Company:      company,
				Namespace:    namespace,
				AgentName:    agentName,
				ConnectionId: connID,
				AgentId:      wrapperspb.UInt64(agentID),
			}

			msg := &grpcapi.ControlMessage{
				MessageId: uuid.NewString(),
				Payload: &grpcapi.ControlMessage_ConfigCommand{
					ConfigCommand: &grpcapi.ConfigurationCommand{
						ConnectionsToCreate: []*grpcapi.Connection{},
						RoutesToSet:         []*grpcapi.Route{},
						RoutesToDelete:      []*grpcapi.Route{route},
					},
				},
			}

			return sendConfigMessage(cmd.Context(), opts, msg)
		},
	}
	return cmd
}

func parseRoute(route string) (
	company,
	namespace,
	agentName string,
	agentID uint64,
	err error,
) {
	parts := strings.Split(route, "/")

	if len(parts) != 4 {
		err = fmt.Errorf(
			"invalid route format '%s', expected 'company/namespace/agentname/agentid'",
			route,
		)
		return
	}

	if parts[0] == "" || parts[1] == "" || parts[2] == "" || parts[3] == "" {
		err = fmt.Errorf(
			"invalid route format '%s', expected 'company/namespace/agentname/agentid'",
			route,
		)
		return
	}

	company = parts[0]
	namespace = parts[1]
	agentName = parts[2]

	agentID, err = strconv.ParseUint(parts[3], 10, 64)
	if err != nil {
		err = fmt.Errorf("invalid agent ID %s", parts[3])
		return
	}

	return
}

func parseEndpoint(endpoint string) (*grpcapi.Connection, string, error) {
	host, portStr, err := net.SplitHostPort(endpoint)
	if err != nil {
		return nil, "", fmt.Errorf(
			"cannot split endpoint '%s' into host:port: %w",
			endpoint,
			err,
		)
	}

	if host == "" {
		return nil, "", fmt.Errorf(
			"invalid endpoint format '%s': host part is missing",
			endpoint,
		)
	}

	port, err := strconv.ParseInt(portStr, 10, 32)
	if err != nil {
		return nil, "", fmt.Errorf(
			"invalid port '%s' in endpoint '%s': %w",
			portStr,
			endpoint,
			err,
		)
	}
	if port <= 0 || port > 65535 {
		return nil, "", fmt.Errorf(
			"port number '%d' in endpoint '%s' out of range (1-65535)",
			port,
			endpoint,
		)
	}

	connID := endpoint
	conn := &grpcapi.Connection{
		ConnectionId:  connID,
		RemoteAddress: host,
		RemotePort:    int32(port),
	}

	return conn, connID, nil
}

func sendConfigMessage(
	ctx context.Context,
	opts *options.CommonOptions,
	msg *grpcapi.ControlMessage,
) error {
	ack, err := controller.SendConfigMessage(ctx, opts, msg)
	if err != nil {
		return fmt.Errorf("failed to send command: %w", err)
	}

	a := ack.GetAck()
	if a == nil {
		return fmt.Errorf("unexpected response type received (not an ACK): %v", ack)
	}

	fmt.Printf("ACK received for %s: success=%t\n", a.OriginalMessageId, a.Success)

	if len(a.Messages) > 0 {
		for i, ackMsg := range a.Messages {
			fmt.Printf("    [%d] %s\n", i+1, ackMsg)
		}
	}

	return nil
}
