package nbapiservice

import (
	"context"
	"fmt"

	"github.com/google/uuid"

	"github.com/agntcy/slim/control-plane/common/controller"
	"github.com/agntcy/slim/control-plane/common/options"
	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
)

type ConfigService struct{}

func NewConfigService() *ConfigService {
	return &ConfigService{}
}

func (s *ConfigService) ModifyConfiguration(
	ctx context.Context, configCommand *controllerapi.ConfigurationCommand, opts *options.CommonOptions) (
	*controllerapi.Ack, error,
) {
	stream, err := controller.OpenControlChannel(ctx, opts)
	if err != nil {
		return nil, fmt.Errorf("failed to open control channel: %w", err)
	}
	messageID := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageID,
		Payload: &controllerapi.ControlMessage_ConfigCommand{
			ConfigCommand: configCommand,
		},
	}

	if err = stream.Send(msg); err != nil {
		return nil, fmt.Errorf("failed to send control message: %w", err)
	}

	if err = stream.CloseSend(); err != nil {
		return nil, fmt.Errorf("failed to close send: %w", err)
	}

	ack, err := stream.Recv()
	if err != nil {
		return nil, fmt.Errorf("error receiving ack via stream: %w", err)
	}

	a := ack.GetAck()
	if a == nil {
		return nil, fmt.Errorf("unexpected response type received (not an ACK): %v", ack)
	}

	fmt.Printf(
		"ACK received for %s: success=%t\n",
		a.OriginalMessageId,
		a.Success,
	)
	if len(a.Messages) > 0 {
		for i, ackMsg := range a.Messages {
			fmt.Printf("    [%d] %s\n", i+1, ackMsg)
		}
	}
	return &controllerapi.Ack{
		OriginalMessageId: messageID,
		Success:           true,
		Messages:          []string{"Configuration command processed successfully"},
	}, nil
}
