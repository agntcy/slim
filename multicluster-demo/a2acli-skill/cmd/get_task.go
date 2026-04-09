// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package cmd

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/a2aproject/a2a-go/v2/a2a"
	"github.com/a2aproject/a2a-go/v2/a2aclient"
	"github.com/spf13/cobra"
)

var getTaskAgentDigest string

var getTaskCmd = &cobra.Command{
	Use:   "get-task --agent <digest> <task-id>",
	Short: "Get the current state of a task",
	Long: `Retrieve a task by ID from the agent identified by the given digest.
Prints the task's status, artifacts, and history as JSON.`,
	Example: `  a2acli get-task --agent sha256:3f7a2c1b "01938f4a-1234-7abc-def0-123456789abc"`,
	Args:    cobra.ExactArgs(1),
	PreRunE: func(cmd *cobra.Command, args []string) error { return initSLIM() },
	RunE: func(cmd *cobra.Command, args []string) error {
		if getTaskAgentDigest == "" {
			return fmt.Errorf("--agent flag is required")
		}
		taskID := a2a.TaskID(args[0])

		agent, err := agentStore.Lookup(getTaskAgentDigest)
		if err != nil {
			return err
		}

		ctx := context.Background()
		client, err := newA2AClient(ctx, agent.Card)
		if err != nil {
			return fmt.Errorf("creating client for agent %q: %w", agent.Card.Name, err)
		}
		defer client.Destroy()

		task, err := client.GetTask(ctx, &a2a.GetTaskRequest{ID: taskID})
		if err != nil {
			return fmt.Errorf("getting task %q: %w", taskID, err)
		}

		return printTask(cmd, client, task)
	},
}

func init() {
	getTaskCmd.Flags().StringVar(&getTaskAgentDigest, "agent", "", "agent digest (full or prefix)")
}

// printTask prints a task as formatted JSON.
func printTask(cmd *cobra.Command, _ *a2aclient.Client, task *a2a.Task) error {
	out, err := json.MarshalIndent(task, "", "  ")
	if err != nil {
		return fmt.Errorf("marshaling task: %w", err)
	}
	fmt.Fprintf(cmd.OutOrStdout(), "%s\n", out)
	return nil
}
