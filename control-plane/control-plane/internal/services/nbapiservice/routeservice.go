package nbapiservice

import (
	"context"
	"fmt"

	"github.com/agntcy/slim/control-plane/common/controller"
	"github.com/agntcy/slim/control-plane/common/options"
	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	"github.com/google/uuid"
)

type routeService struct{}

func NewRouteService() *routeService {
	return &routeService{}
}

func (s *routeService) ListSubscriptions(ctx context.Context, opts *options.CommonOptions) (*controllerapi.SubscriptionListResponse, error) {
	msg := &controllerapi.ControlMessage{
		MessageId: uuid.NewString(),
		Payload:   &controllerapi.ControlMessage_SubscriptionListRequest{},
	}

	stream, err := controller.OpenControlChannel(ctx, opts)
	if err != nil {
		return nil, fmt.Errorf("failed to open control channel: %w", err)
	}

	if err := stream.Send(msg); err != nil {
		return nil, fmt.Errorf("failed to send control message: %w", err)
	}

	if err := stream.CloseSend(); err != nil {
		return nil, fmt.Errorf("failed to close send: %w", err)
	}

	for {
		resp, err := stream.Recv()
		if err != nil {
			break
		}

		if listResp := resp.GetSubscriptionListResponse(); listResp != nil {
			for _, e := range listResp.Entries {
				var localNames, remoteNames []string
				for _, lc := range e.GetLocalConnections() {
					localNames = append(localNames,
						fmt.Sprintf("local:%d:%s", lc.GetId(), lc.GetConfigData()))
				}
				for _, rc := range e.GetRemoteConnections() {
					remoteNames = append(remoteNames,
						fmt.Sprintf("remote:%d:%s", rc.GetId(), rc.GetConfigData()))
				}
				fmt.Printf("%s/%s/%s id=%d local=%v remote=%v\n",
					e.GetOrganization(), e.GetNamespace(), e.GetAgentType(),
					e.GetAgentId().GetValue(),
					localNames, remoteNames,
				)
			}
			return listResp, nil
		}
	}
	return nil, fmt.Errorf("no SubscriptionListResponse found in response")
}

func (s *routeService) ListConnections(ctx context.Context, opts *options.CommonOptions) (*controllerapi.ConnectionListResponse, error) {
	msg := &controllerapi.ControlMessage{
		MessageId: uuid.NewString(),
		Payload:   &controllerapi.ControlMessage_ConnectionListRequest{},
	}

	stream, err := controller.OpenControlChannel(ctx, opts)
	if err != nil {
		return nil, fmt.Errorf("open control channel: %w", err)
	}

	if err := stream.Send(msg); err != nil {
		return nil, fmt.Errorf("send request: %w", err)
	}
	if err := stream.CloseSend(); err != nil {
		return nil, fmt.Errorf("close send: %w", err)
	}

	for {
		resp, err := stream.Recv()
		if err != nil {
			break
		}
		fmt.Printf("Received response: %v\n", resp)
		if listResp := resp.GetConnectionListResponse(); listResp != nil {
			for _, e := range listResp.Entries {
				fmt.Printf("id=%d %s:%d\n",
					e.GetId(),
					e.GetConfigData(),
				)
			}
			return listResp, nil
		}
	}
	return nil, fmt.Errorf("no ConnectionListResponse found in response")
}

func (s *routeService) CreateConnection(ctx context.Context, connection *controllerapi.Connection, opts *options.CommonOptions) error {
	controllerConfigCommand := &controllerapi.ConfigurationCommand{
		ConnectionsToCreate:   []*controllerapi.Connection{connection},
		SubscriptionsToSet:    []*controllerapi.Subscription{},
		SubscriptionsToDelete: []*controllerapi.Subscription{},
	}

	messageId := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageId,
		Payload: &controllerapi.ControlMessage_ConfigCommand{
			ConfigCommand: controllerConfigCommand,
		},
	}

	stream, err := controller.OpenControlChannel(ctx, opts)
	if err != nil {
		return fmt.Errorf("failed to open control channel: %w", err)
	}
	if err = stream.Send(msg); err != nil {
		return fmt.Errorf("failed to send control message: %w", err)
	}
	if err = stream.CloseSend(); err != nil {
		return fmt.Errorf("failed to close send: %w", err)
	}
	ack, err := stream.Recv()
	if err != nil {
		return fmt.Errorf("error receiving ack via stream: %w", err)
	}
	a := ack.GetAck()
	if a == nil {
		return fmt.Errorf("unexpected response type received (not an ACK): %v", ack)
	}
	return nil
}

func (s *routeService) CreateSubscription(ctx context.Context, subscription *controllerapi.Subscription, opts *options.CommonOptions) error {
	controllerConfigCommand := &controllerapi.ConfigurationCommand{
		ConnectionsToCreate:   []*controllerapi.Connection{},
		SubscriptionsToSet:    []*controllerapi.Subscription{subscription},
		SubscriptionsToDelete: []*controllerapi.Subscription{},
	}

	messageId := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageId,
		Payload: &controllerapi.ControlMessage_ConfigCommand{
			ConfigCommand: controllerConfigCommand,
		},
	}

	stream, err := controller.OpenControlChannel(ctx, opts)
	if err != nil {
		return fmt.Errorf("failed to open control channel: %w", err)
	}
	if err = stream.Send(msg); err != nil {
		return fmt.Errorf("failed to send control message: %w", err)
	}
	if err = stream.CloseSend(); err != nil {
		return fmt.Errorf("failed to close send: %w", err)
	}
	ack, err := stream.Recv()
	if err != nil {
		return fmt.Errorf("error receiving ack via stream: %w", err)
	}
	a := ack.GetAck()
	if a == nil {
		return fmt.Errorf("unexpected response type received (not an ACK): %v", ack)
	}
	return nil
}

func (s *routeService) DeleteSubscription(ctx context.Context, subscription *controllerapi.Subscription, opts *options.CommonOptions) error {
	msg := &controllerapi.ControlMessage{
		MessageId: uuid.NewString(),
		Payload: &controllerapi.ControlMessage_ConfigCommand{
			ConfigCommand: &controllerapi.ConfigurationCommand{
				ConnectionsToCreate:   []*controllerapi.Connection{},
				SubscriptionsToSet:    []*controllerapi.Subscription{},
				SubscriptionsToDelete: []*controllerapi.Subscription{subscription},
			},
		},
	}

	stream, err := controller.OpenControlChannel(ctx, opts)
	if err != nil {
		return fmt.Errorf("failed to open control channel: %w", err)
	}

	if err = stream.Send(msg); err != nil {
		return fmt.Errorf("failed to send control message: %w", err)
	}

	if err = stream.CloseSend(); err != nil {
		return fmt.Errorf("failed to close send: %w", err)
	}

	ack, err := stream.Recv()
	if err != nil {
		return fmt.Errorf("error receiving ack via stream: %w", err)
	}

	a := ack.GetAck()
	if a == nil {
		return fmt.Errorf("unexpected response type received (not an ACK): %v", ack)
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
	return nil
}
