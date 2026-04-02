// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package cmd

import (
	"encoding/json"
	"fmt"

	"github.com/spf13/cobra"
)

var getCardAgentDigest string

var getCardCmd = &cobra.Command{
	Use:   "get-card --agent <digest>",
	Short: "Get the full AgentCard for an agent",
	Long: `Print the full AgentCard JSON for the agent identified by the given digest.
The digest can be the full SHA256 digest (e.g. sha256:3f7a2c1b...) or an
unambiguous prefix.`,
	Example: `  a2acli get-card --agent sha256:3f7a2c1b
  a2acli get-card --agent sha256:3f7a2c1b4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a`,
	RunE: func(cmd *cobra.Command, args []string) error {
		if getCardAgentDigest == "" {
			return fmt.Errorf("--agent flag is required")
		}

		agent, err := agentStore.Lookup(getCardAgentDigest)
		if err != nil {
			return err
		}

		out, err := json.MarshalIndent(agent.Card, "", "  ")
		if err != nil {
			return fmt.Errorf("marshaling AgentCard: %w", err)
		}

		fmt.Fprintf(cmd.OutOrStdout(), "%s\n", out)
		return nil
	},
}

func init() {
	getCardCmd.Flags().StringVar(&getCardAgentDigest, "agent", "", "agent digest (full or prefix)")
}
