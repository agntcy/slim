// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package controller

import (
	"context"
	"fmt"
	"os"
	"strings"
	"text/tabwriter"
	"time"

	"github.com/spf13/cobra"
	"google.golang.org/protobuf/types/known/wrapperspb"

	cpApi "github.com/agntcy/slim/control-plane/common/controlplane"
	"github.com/agntcy/slim/control-plane/common/options"
	grpcapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	commonUtil "github.com/agntcy/slim/control-plane/common/util"
	"github.com/agntcy/slim/control-plane/slimctl/internal/cmd/util"
)

const (
	nodeIDFlag       = "node-id"
	targetNodeIDFlag = "target-node-id"
	originNodeIDFlag = "origin-node-id"
)

func NewRouteCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "route",
		Short: "Manage SLIM routes",
		Long:  `Manage SLIM routes`,
	}

	cmd.AddCommand(markNodeIDRequired(newListSubscriptionsCmd(opts)))
	cmd.AddCommand(markNodeIDRequired(newAddCmd(opts)))
	cmd.AddCommand(markNodeIDRequired(newDelCmd(opts)))
	cmd.AddCommand(newOutlineRoutesCmd(opts))

	return cmd
}

func newListSubscriptionsCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "list",
		Short: "List subscriptions",
		Long:  `List subscriptions for a SLIM instance`,
		RunE: func(cmd *cobra.Command, _ []string) error {
			nodeID, _ := cmd.Flags().GetString(nodeIDFlag)
			fmt.Printf("Listing routes for node ID: %s\n", nodeID)
			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpClient, conn, ctx, err := cpApi.GetClient(ctx, opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}
			defer conn.Close()

			subscriptionListResponse, err := cpClient.ListSubscriptions(ctx, &controlplaneApi.Node{
				Id: nodeID,
			})
			if err != nil {
				return fmt.Errorf("failed to list subscriptions: %w", err)
			}

			fmt.Printf("Received connection list response: %v\n", len(subscriptionListResponse.Entries))
			for _, e := range subscriptionListResponse.Entries {
				var localNames, remoteNames []string
				for _, c := range e.LocalConnections {
					localNames = append(localNames,
						fmt.Sprintf("local:%d", c.Id))
				}
				for _, c := range e.RemoteConnections {
					remoteNames = append(remoteNames,
						fmt.Sprintf("remote:%s:%v:%d", c.ConnectionType, c.ConfigData, c.Id))
				}
				fmt.Printf("%s/%s/%s id=%v local=%v remote=%v\n",
					e.Component_0, e.Component_1, e.Component_2,
					e.Id,
					localNames, remoteNames,
				)
			}

			return nil
		},
	}
	return cmd
}

func newAddCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "add <organization/namespace/agentname/agentid> via <slim-node-id or path_to_config_file>",
		Short: "Add a route to a SLIM instance",
		Long:  `Add a route to a SLIM instance`,
		Args:  cobra.ExactArgs(3),
		RunE: func(cmd *cobra.Command, args []string) error {
			routeID := args[0]
			viaKeyword := strings.ToLower(args[1])
			configFile := args[2]

			nodeID, _ := cmd.Flags().GetString(nodeIDFlag)
			fmt.Printf("Add route for node ID: %s\n", nodeID)

			if viaKeyword != "via" {
				return fmt.Errorf(
					"invalid syntax: expected 'via' keyword, got '%s'",
					args[1],
				)
			}

			organization, namespace, agentType, agentID, err := commonUtil.ParseRoute(routeID)
			if err != nil {
				return err
			}

			destNodeID := ""
			var connection *grpcapi.Connection
			// check if config file exists
			if _, err = os.Stat(configFile); err != nil {
				destNodeID = configFile
			} else {
				connection, err = util.ParseConfigFile(configFile)
				if err != nil {
					return err
				}
			}

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpClient, conn, ctx, err := cpApi.GetClient(ctx, opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}
			defer conn.Close()

			subscription := &grpcapi.Subscription{
				Component_0: organization,
				Component_1: namespace,
				Component_2: agentType,
				Id:          wrapperspb.UInt64(agentID),
			}

			addRouteRequest := controlplaneApi.AddRouteRequest{
				NodeId:       nodeID,
				Subscription: subscription,
			}
			if connection != nil {
				subscription.ConnectionId = connection.ConnectionId
				addRouteRequest.Connection = connection
			} else {
				addRouteRequest.DestNodeId = destNodeID
			}

			addRouteResponse, err := cpClient.AddRoute(ctx, &addRouteRequest)
			if err != nil {
				return fmt.Errorf("failed to create route: %w", err)
			}
			if !addRouteResponse.Success {
				return fmt.Errorf("failed to create route")
			}
			fmt.Printf("Route created successfully with ID: %v\n", addRouteResponse.RouteId)

			return nil
		},
	}
	return cmd
}

func newDelCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "del <organization/namespace/agentname/agentid> via <slim-node-id or http|https://host:port>",
		Short: "Delete a route from a SLIM instance",
		Long:  `Delete a route from a SLIM instance`,
		Args:  cobra.ExactArgs(3),
		RunE: func(cmd *cobra.Command, args []string) error {
			routeID := args[0]
			viaKeyword := strings.ToLower(args[1])
			endpoint := args[2]

			nodeID, _ := cmd.Flags().GetString(nodeIDFlag)
			fmt.Printf("Delete route for node ID: %s\n", nodeID)

			if viaKeyword != "via" {
				return fmt.Errorf(
					"invalid syntax: expected 'via' keyword, got '%s'",
					args[1],
				)
			}

			organization, namespace, agentType, agentID, err := commonUtil.ParseRoute(routeID)
			if err != nil {
				return err
			}

			subscription := &grpcapi.Subscription{
				Component_0: organization,
				Component_1: namespace,
				Component_2: agentType,
				Id:          wrapperspb.UInt64(agentID),
			}

			deleteRouteRequest := &controlplaneApi.DeleteRouteRequest{
				NodeId:       nodeID,
				Subscription: subscription,
			}

			// determine if endpoint is a node ID or a connection ID
			if util.IsEndpoint(endpoint) {
				_, connID, err2 := util.ParseEndpoint(endpoint)
				if err2 != nil {
					return fmt.Errorf("invalid endpoint format '%s': %w", endpoint, err)
				}
				subscription.ConnectionId = connID
			} else {
				deleteRouteRequest.DestNodeId = endpoint
			}

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpClient, conn, ctx, err := cpApi.GetClient(ctx, opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}
			defer conn.Close()

			returnedMessage, err := cpClient.DeleteRoute(ctx, deleteRouteRequest)
			if err != nil {
				return fmt.Errorf("failed to delete route: %w", err)
			}

			fmt.Printf(
				"ACK received success=%v\n",
				returnedMessage.Success,
			)

			return nil
		},
	}
	return cmd
}

func newOutlineRoutesCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "outline",
		Short: "List routes from the controller",
		Long:  `List routes from the controller eventually filtered by origin node ID and/or target node ID`,
		RunE: func(cmd *cobra.Command, _ []string) error {
			srcNodeID, _ := cmd.Flags().GetString(originNodeIDFlag)
			destNodeID, _ := cmd.Flags().GetString(targetNodeIDFlag)
			fmt.Printf("Outline routes (origin:[%s] target:[%s]) \n", srcNodeID, destNodeID)

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpClient, conn, ctx, err := cpApi.GetClient(ctx, opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}
			defer conn.Close()

			outlineRoutesResponse, err := cpClient.ListRoutes(ctx, &controlplaneApi.RouteListRequest{
				SrcNodeId:  srcNodeID,
				DestNodeId: destNodeID,
			})
			if err != nil {
				return fmt.Errorf("failed to outline routes: %w", err)
			}

			routes := outlineRoutesResponse.GetRoutes()
			fmt.Printf("Number of routes: %v\n\n", len(routes))

			if len(routes) == 0 {
				return nil
			}

			// Create tabwriter for aligned output
			w := tabwriter.NewWriter(os.Stdout, 0, 0, 2, ' ', 0)
			defer w.Flush()

			// Print header
			fmt.Fprintln(w, "ID\tSOURCE\tDEST_NODE\tDEST_ENDPOINT\tSUBSCRIPTION\tSTATUS\tDELETED\tLAST_UPDATED")
			fmt.Fprintln(w, strings.Repeat("-", 150))

			// Print routes (already sorted: * routes first, then by srcNodeID)
			for _, route := range routes {
				subscription := fmt.Sprintf("%s/%s/%s", route.Component_0, route.Component_1, route.Component_2)
				if route.ComponentId != nil {
					subscription = fmt.Sprintf("%s/%d", subscription, *route.ComponentId)
				}

				status := "UNKNOWN"
				switch route.Status {
				case controlplaneApi.RouteStatus_ROUTE_STATUS_APPLIED:
					status = "APPLIED"
				case controlplaneApi.RouteStatus_ROUTE_STATUS_FAILED:
					status = "FAILED"
				}

				lastUpdated := time.Unix(route.LastUpdated, 0).Format(time.RFC3339)

				destNode := route.DestNodeId
				if destNode == "" {
					destNode = "-"
				}

				destEndpoint := route.DestEndpoint
				if destEndpoint == "" {
					destEndpoint = "-"
				}

				deleted := "No"
				if route.Deleted {
					deleted = "Yes"
				}

				fmt.Fprintf(w, "%d\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n",
					route.Id,
					route.SourceNodeId,
					destNode,
					destEndpoint,
					subscription,
					status,
					deleted,
					lastUpdated,
				)
			}

			return nil
		},
	}
	cmd.PersistentFlags().StringP(originNodeIDFlag, "o", "", "ID of the route origin (node)")
	cmd.PersistentFlags().StringP(targetNodeIDFlag, "t", "", "ID of the route target (node)")
	return cmd
}

// markNodeIDRequired marks the node-id flag as required for the given cobra command.
func markNodeIDRequired(cmd *cobra.Command) *cobra.Command {
	cmd.PersistentFlags().StringP(nodeIDFlag, "n", "", "ID of the node to manage routes for")
	err := cmd.MarkPersistentFlagRequired(nodeIDFlag)
	if err != nil {
		fmt.Printf("Error marking node-id flag as required: %v\n", err)
	}

	return cmd
}
