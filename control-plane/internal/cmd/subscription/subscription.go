package subscription

import (
	"context"
	"fmt"
	"time"

	"github.com/spf13/cobra"
	"google.golang.org/grpc"

	"github.com/agntcy/agp/control-plane/internal/options"
	controllerv1 "github.com/agntcy/agp/control-plane/internal/proto/controller/v1"
)

var serverAddr string

// NewSubscribeCmd returns a CLI command to send a subscribe command.
func NewSubscribeCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "subscribe",
		Short: "Send a subscribe command to the gateway",
		RunE: func(cmd *cobra.Command, args []string) error {
			return sendCommand("subscribe")
		},
	}
	cmd.Flags().StringVarP(&serverAddr, "server", "s", "localhost:46357", "Gateway control API address")
	return cmd
}

// NewUnsubscribeCmd returns a CLI command to send an unsubscribe command.
func NewUnsubscribeCmd(opts *options.CommonOptions) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "unsubscribe",
		Short: "Send an unsubscribe command to the gateway",
		RunE: func(cmd *cobra.Command, args []string) error {
			return sendCommand("unsubscribe")
		},
	}
	cmd.Flags().StringVarP(&serverAddr, "server", "s", "localhost:46357", "Gateway control API address")
	return cmd
}

func sendCommand(cmdType string) error {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	conn, err := grpc.DialContext(ctx, serverAddr, grpc.WithInsecure(), grpc.WithBlock())
	if err != nil {
		return fmt.Errorf("failed to dial server: %v", err)
	}
	defer conn.Close()

	client := controllerv1.NewControllerServiceClient(conn)
	stream, err := client.OpenChannel(ctx)
	if err != nil {
		return fmt.Errorf("failed to open channel: %v", err)
	}

	var command *controllerv1.Command
	switch cmdType {
	case "subscribe":
		command = &controllerv1.Command{
			CommandType: &controllerv1.Command_Subscribe{
				Subscribe: &controllerv1.Subscribe{},
			},
			Metadata: map[string]string{},
		}
	case "unsubscribe":
		command = &controllerv1.Command{
			CommandType: &controllerv1.Command_Unsubscribe{
				Unsubscribe: &controllerv1.Unsubscribe{},
			},
			Metadata: map[string]string{},
		}
	default:
		return fmt.Errorf("unknown command type: %s", cmdType)
	}

	if err := stream.Send(command); err != nil {
		return fmt.Errorf("failed to send %s command: %v", cmdType, err)
	}

	resp, err := stream.Recv()
	if err != nil {
		return fmt.Errorf("failed to receive response: %v", err)
	}

	fmt.Printf("Received response: %+v\n", resp)
	return nil
}
