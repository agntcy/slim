// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Package cmd implements the a2acli command-line interface.
package cmd

import (
	"context"
	"fmt"
	"os"

	"github.com/a2aproject/a2a-go/v2/a2a"
	"github.com/a2aproject/a2a-go/v2/a2aclient"
	a2aslimrpcv0 "github.com/agntcy/slim-a2a-go/a2aslimrpc/v0"
	a2aslimrpcv1 "github.com/agntcy/slim-a2a-go/a2aslimrpc/v1"
	slim_bindings "github.com/agntcy/slim-bindings-go"
	"github.com/spf13/cobra"
	"github.com/tehsmash/a2acli/internal/config"
	"github.com/tehsmash/a2acli/internal/store"
)

var (
	configFile string
	agentsDir  string
	agentStore *store.Store

	slimEndpoint  string
	slimLocalName string
	slimSecret    string
	slimApp       *slim_bindings.App
	slimConnID    uint64
)

var rootCmd = &cobra.Command{
	Use:   "a2acli",
	Short: "A CLI for interacting with A2A protocol agents",
	Long: `a2acli lets you list, inspect, and communicate with agents that
expose the A2A (Agent-to-Agent) protocol.

AgentCard JSON files are read from a local .a2aagents directory.
Each agent is identified by the SHA256 digest of its AgentCard file.`,
	PersistentPreRunE: func(cmd *cobra.Command, args []string) error {
		cfg, err := config.Load(configFile)
		if err != nil {
			return err
		}

		flags := cmd.Root().PersistentFlags()
		if !flags.Changed("agents-dir") && cfg.AgentsDir != "" {
			agentsDir = cfg.AgentsDir
		}
		if !flags.Changed("slim-endpoint") && cfg.Slim.Endpoint != "" {
			slimEndpoint = cfg.Slim.Endpoint
		}
		if !flags.Changed("slim-local-name") && cfg.Slim.LocalName != "" {
			slimLocalName = cfg.Slim.LocalName
		}
		if !flags.Changed("slim-secret") && cfg.Slim.Secret != "" {
			slimSecret = cfg.Slim.Secret
		}

		s, err := store.NewStore(agentsDir)
		if err != nil {
			return err
		}
		agentStore = s

		if slimEndpoint != "" {
			slim_bindings.InitializeWithDefaults()
			svc := slim_bindings.GetGlobalService()

			localName, err := slim_bindings.NameFromString(slimLocalName)
			if err != nil {
				return fmt.Errorf("invalid --slim-local-name %q: %w", slimLocalName, err)
			}

			app, err := svc.CreateAppWithSecret(localName, slimSecret)
			if err != nil {
				return fmt.Errorf("SLIM CreateApp failed: %w", err)
			}
			slimApp = app

			connID, err := svc.Connect(slim_bindings.NewInsecureClientConfig(slimEndpoint))
			if err != nil {
				return fmt.Errorf("SLIM Connect failed: %w", err)
			}
			slimConnID = connID

			if err := app.Subscribe(localName, &slimConnID); err != nil {
				return fmt.Errorf("SLIM Subscribe failed: %w", err)
			}
		}

		return nil
	},
}

// newA2AClient creates an A2A client from an AgentCard, registering the SLIM
// RPC transport when a SLIM endpoint has been configured via --slim-endpoint.
func newA2AClient(ctx context.Context, card *a2a.AgentCard) (*a2aclient.Client, error) {
	opts := []a2aclient.FactoryOption{}
	if slimApp != nil {
		opts = append(opts, a2aslimrpcv1.WithSLIMRPCTransport(slimApp, &slimConnID))
		opts = append(opts, a2aslimrpcv0.WithSLIMRPCTransport(slimApp, &slimConnID))
	}
	return a2aclient.NewFromCard(ctx, card, opts...)
}

// Execute runs the root command.
func Execute() {
	if err := rootCmd.Execute(); err != nil {
		os.Exit(1)
	}
}

func init() {
	rootCmd.PersistentFlags().StringVar(&configFile, "config", "", "path to config file (default: .a2acli.yaml in current dir, then ~/.a2acli.yaml)")
	rootCmd.PersistentFlags().StringVar(&agentsDir, "agents-dir", "", "path to the agents directory (default: .a2aagents in current dir, then ~/.a2aagents)")
	rootCmd.PersistentFlags().StringVar(&slimEndpoint, "slim-endpoint", "", "SLIM node URL (e.g. http://localhost:46357)")
	rootCmd.PersistentFlags().StringVar(&slimLocalName, "slim-local-name", "agntcy/cli/a2acli", "local SLIM name (namespace/group/name)")
	rootCmd.PersistentFlags().StringVar(&slimSecret, "slim-secret", "", "shared secret for SLIM authentication")
	rootCmd.AddCommand(listCmd)
	rootCmd.AddCommand(getCardCmd)
	rootCmd.AddCommand(sendMessageCmd)
	rootCmd.AddCommand(getTaskCmd)
}
