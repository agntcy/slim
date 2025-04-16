// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package configure

import (
	"context"
	"fmt"
	"strconv"
	"strings"

	"github.com/google/uuid"
	"github.com/spf13/cobra"
	"google.golang.org/protobuf/types/known/wrapperspb"

	"github.com/agntcy/agp/control-plane/internal/controller"
	"github.com/agntcy/agp/control-plane/internal/options"
	grpcapi "github.com/agntcy/agp/control-plane/internal/proto/controller/v1"
)

func NewConfigureCmd(opts *options.CommonOptions) *cobra.Command {
	var serverAddr string
	var conns []string
	var setRoutes []string
	var delRoutes []string

	cmd := &cobra.Command{
		Use:   "configure",
		Short: "Send a configuration command to a gateway",
		RunE: func(cmd *cobra.Command, args []string) error {
			var connections []*grpcapi.Connection
			for _, c := range conns {
				parts := strings.Split(c, ",")
				if len(parts) != 3 {
					return fmt.Errorf("invalid connection format '%s', expected id,address,port", c)
				}
				port, err := strconv.Atoi(parts[2])
				if err != nil {
					return fmt.Errorf("invalid port in '%s': %w", c, err)
				}
				connections = append(connections, &grpcapi.Connection{
					ConnectionId:  parts[0],
					RemoteAddress: parts[1],
					RemotePort:    int32(port),
				})
			}

			var routesSet []*grpcapi.Route
			for _, r := range setRoutes {
				rt, err := parseRoute(r)
				if err != nil {
					return err
				}
				routesSet = append(routesSet, rt)
			}

			var routesDel []*grpcapi.Route
			for _, r := range delRoutes {
				rt, err := parseRoute(r)
				if err != nil {
					return err
				}
				routesDel = append(routesDel, rt)
			}

			msg := &grpcapi.ControlMessage{
				MessageId: uuid.NewString(),
				Payload: &grpcapi.ControlMessage_ConfigCommand{ConfigCommand: &grpcapi.ConfigurationCommand{
					ConnectionsToCreate: connections,
					RoutesToSet:         routesSet,
					RoutesToDelete:      routesDel,
				}},
			}

			ack, err := controller.SendConfigMessage(context.Background(), serverAddr, msg)
			if err != nil {
				return err
			}

			if a := ack.GetAck(); a != nil {
				fmt.Printf("ACK for %s: success=%v\n", a.OriginalMessageId, a.Success)
			} else {
				fmt.Printf("unexpected response: %v\n", ack)
			}
			return nil
		},
	}

	cmd.Flags().StringVarP(&serverAddr, "server", "s", "localhost:46357", "gateway gRPC control API endpoint (host:port)")
	cmd.Flags().StringArrayVar(&conns, "connection", nil, "id,address,port (repeatable)")
	cmd.Flags().StringArrayVar(&setRoutes, "route-set", nil, "company,namespace,agent,agent_id(optional),conn_id (repeatable)")
	cmd.Flags().StringArrayVar(&delRoutes, "route-delete", nil, "company,namespace,agent,agent_id(optional),conn_id (repeatable)")

	return cmd
}

func parseRoute(s string) (*grpcapi.Route, error) {
	parts := strings.Split(s, ",")
	if len(parts) < 4 || len(parts) > 5 {
		return nil, fmt.Errorf("invalid route format '%s', expected company,namespace,agent,agent_id?,conn_id", s)
	}

	rt := &grpcapi.Route{
		Company:      parts[0],
		Namespace:    parts[1],
		AgentName:    parts[2],
		ConnectionId: parts[len(parts)-1],
	}
	if len(parts) == 5 {
		id, err := strconv.ParseUint(parts[3], 10, 64)
		if err != nil {
			return nil, fmt.Errorf("invalid agent_id '%s': %w", parts[3], err)
		}
		rt.AgentId = &wrapperspb.UInt64Value{Value: id}
	}

	return rt, nil
}
