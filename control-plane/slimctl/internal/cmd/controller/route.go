// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package controller

import (
	"context"
	"fmt"
	"strings"

	"github.com/spf13/cobra"
	"google.golang.org/protobuf/types/known/wrapperspb"

	cpApi "github.com/agntcy/slim/control-plane/common/controlplane"
	"github.com/agntcy/slim/control-plane/common/options"
	grpcapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/slimctl/internal/cmd/util"
)

const nodeIDFlag = "node-id"

func NewRouteCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "route",
		Short: "Manage SLIM routes",
		Long:  `Manage SLIM routes`,
	}

	cmd.PersistentFlags().StringP(nodeIDFlag, "n", "", "ID of the node to manage routes for")

	err := cmd.MarkPersistentFlagRequired(nodeIDFlag)
	if err != nil {
		fmt.Printf("Error marking persistent flag required: %v\n", err)
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
			nodeID, _ := cmd.Flags().GetString(nodeIDFlag)
			fmt.Printf("Listing routes for node ID: %s\n", nodeID)

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpCLient, err := cpApi.GetClient(opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}

			subscriptionListResponse, err := cpCLient.ListSubscriptions(ctx, &controlplaneApi.Node{
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
					e.Organization, e.Namespace, e.AgentType,
					e.AgentId,
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
		Use:   "add <organization/namespace/agentname/agentid> via <config_file>",
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

			organization, namespace, agentType, agentID, err := util.ParseRoute(routeID)
			if err != nil {
				return err
			}

			conn, err := util.ParseConfigFile(configFile)
			if err != nil {
				return err
			}

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpClient, err := cpApi.GetClient(opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}

			createConnectionResponse, err := cpClient.CreateConnection(ctx, &controlplaneApi.CreateConnectionRequest{
				NodeId:     nodeID,
				Connection: conn,
			})
			if err != nil {
				return fmt.Errorf("failed to create connection: %w", err)
			}
			connectionID := createConnectionResponse.ConnectionId
			if !createConnectionResponse.Success {
				return fmt.Errorf("failed to create connection")
			}
			fmt.Printf("Connection created successfully with ID: %v\n", connectionID)

			subscription := &grpcapi.Subscription{
				Organization: organization,
				Namespace:    namespace,
				AgentType:    agentType,
				ConnectionId: connectionID,
				AgentId:      wrapperspb.UInt64(agentID),
			}

			createSubscriptionResponse, err := cpClient.CreateSubscription(ctx, &controlplaneApi.CreateSubscriptionRequest{
				NodeId:       nodeID,
				Subscription: subscription,
			})
			if err != nil {
				return fmt.Errorf("failed to create subscription: %w", err)
			}
			if !createSubscriptionResponse.Success {
				return fmt.Errorf("failed to create subscription")
			}
			fmt.Printf("Subscrption created successfully with ID: %v\n", createSubscriptionResponse.SubscriptionId)

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

			nodeID, _ := cmd.Flags().GetString(nodeIDFlag)
			fmt.Printf("Delete route for node ID: %s\n", nodeID)

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

			subscriptionID := cpApi.GetSubscriptionID(subscription)

			ctx, cancel := context.WithTimeout(cmd.Context(), opts.Timeout)
			defer cancel()

			cpCLient, err := cpApi.GetClient(opts)
			if err != nil {
				return fmt.Errorf("failed to get control plane client: %w", err)
			}

			returnedMessage, err := cpCLient.DeleteSubscription(ctx,
				&controlplaneApi.DeleteSubscriptionRequest{
					NodeId:         nodeID,
					SubscriptionId: subscriptionID,
				})
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
