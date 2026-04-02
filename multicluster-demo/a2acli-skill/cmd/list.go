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
	"fmt"
	"strings"
	"text/tabwriter"

	"github.com/spf13/cobra"
)

var listCmd = &cobra.Command{
	Use:   "list",
	Short: "List all available agents",
	Long:  `List all agents found in the agents directory, showing their name, description, and short digest.`,
	RunE: func(cmd *cobra.Command, args []string) error {
		agents, err := agentStore.List()
		if err != nil {
			return err
		}

		if len(agents) == 0 {
			fmt.Printf("No agents found in %s\n", agentStore.Dir)
			return nil
		}

		w := tabwriter.NewWriter(cmd.OutOrStdout(), 0, 0, 3, ' ', 0)
		fmt.Fprintln(w, "NAME\tDESCRIPTION\tDIGEST")
		fmt.Fprintln(w, "----\t-----------\t------")
		for _, agent := range agents {
			desc := agent.Card.Description
			if len(desc) > 60 {
				desc = desc[:57] + "..."
			}
			// Replace any tabs/newlines in description to keep table clean
			desc = strings.ReplaceAll(desc, "\t", " ")
			desc = strings.ReplaceAll(desc, "\n", " ")
			fmt.Fprintf(w, "%s\t%s\t%s\n", agent.Card.Name, desc, agent.ShortDigest())
		}
		return w.Flush()
	},
}
