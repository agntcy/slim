// Copyright 2025 Sam Betts
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

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
