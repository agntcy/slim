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
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/a2aproject/a2a-go/v2/a2a"
	"github.com/a2aproject/a2a-go/v2/a2aclient"
	"github.com/spf13/cobra"
)

var (
	sendMessageAgentDigest   string
	sendMessageReturnImmediately bool
)

var sendMessageCmd = &cobra.Command{
	Use:   "send-message --agent <digest> [--return-immediately] <message>",
	Short: "Send a message to an agent",
	Long: `Send a text message to the agent identified by the given digest.

If the agent responds synchronously with a Message, the response is printed.
If the agent responds with a Task (async), the CLI polls until the task reaches
a terminal state and then prints the result.

Use --return-immediately to instruct the agent to return a Task straight away
without waiting for completion. The Task ID and initial status are printed and
no polling is performed.`,
	Example: `  a2acli send-message --agent sha256:3f7a2c1b "What can you do?"
  a2acli send-message --agent sha256:3f7a2c1b --return-immediately "Run a long analysis"`,
	Args: cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		if sendMessageAgentDigest == "" {
			return fmt.Errorf("--agent flag is required")
		}
		message := args[0]

		agent, err := agentStore.Lookup(sendMessageAgentDigest)
		if err != nil {
			return err
		}

		ctx := context.Background()
		client, err := newA2AClient(ctx, agent.Card)
		if err != nil {
			return fmt.Errorf("creating client for agent %q: %w", agent.Card.Name, err)
		}
		defer client.Destroy()

		req := &a2a.SendMessageRequest{
			Message: a2a.NewMessage(a2a.MessageRoleUser, a2a.NewTextPart(message)),
		}
		if sendMessageReturnImmediately {
			req.Config = &a2a.SendMessageConfig{ReturnImmediately: true}
		}

		result, err := client.SendMessage(ctx, req)
		if err != nil {
			return fmt.Errorf("sending message: %w", err)
		}

		switch r := result.(type) {
		case *a2a.Message:
			printMessage(cmd, r)
		case *a2a.Task:
			if sendMessageReturnImmediately {
				fmt.Fprintf(cmd.OutOrStdout(), "Task ID: %s\nStatus:  %s\n", r.ID, r.Status.State)
				return nil
			}
			return waitForTask(ctx, cmd, client, r.ID)
		}
		return nil
	},
}

func init() {
	sendMessageCmd.Flags().StringVar(&sendMessageAgentDigest, "agent", "", "agent digest (full or prefix)")
	sendMessageCmd.Flags().BoolVar(&sendMessageReturnImmediately, "return-immediately", false, "ask the agent to return a Task immediately without waiting for completion")
}

// printMessage prints all text parts of a Message to the command output.
func printMessage(cmd *cobra.Command, msg *a2a.Message) {
	var parts []string
	for _, part := range msg.Parts {
		if t := part.Text(); t != "" {
			parts = append(parts, t)
		}
	}
	if len(parts) > 0 {
		fmt.Fprintln(cmd.OutOrStdout(), strings.Join(parts, "\n"))
	}
}

// waitForTask polls GetTask every 2 seconds until the task reaches a terminal state.
func waitForTask(ctx context.Context, cmd *cobra.Command, client *a2aclient.Client, taskID a2a.TaskID) error {
	fmt.Fprintf(cmd.OutOrStdout(), "Task ID: %s\nWaiting for task to complete...\n", taskID)

	ticker := time.NewTicker(2 * time.Second)
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-ticker.C:
			task, err := client.GetTask(ctx, &a2a.GetTaskRequest{ID: taskID})
			if err != nil {
				return fmt.Errorf("polling task %s: %w", taskID, err)
			}

			fmt.Fprintf(cmd.OutOrStdout(), "Status: %s\n", task.Status.State)

			if task.Status.State.Terminal() {
				printTaskResult(cmd, task)
				return nil
			}
		}
	}
}

// printTaskResult prints the final state, status message, and artifacts of a completed task.
func printTaskResult(cmd *cobra.Command, task *a2a.Task) {
	out := cmd.OutOrStdout()
	fmt.Fprintf(out, "\nFinal status: %s\n", task.Status.State)

	if task.Status.Message != nil {
		printMessage(cmd, task.Status.Message)
	}

	for i, artifact := range task.Artifacts {
		if artifact.Name != "" {
			fmt.Fprintf(out, "\nArtifact %d: %s\n", i+1, artifact.Name)
		} else {
			fmt.Fprintf(out, "\nArtifact %d:\n", i+1)
		}
		for _, part := range artifact.Parts {
			if t := part.Text(); t != "" {
				fmt.Fprintln(out, t)
			}
		}
	}
}
