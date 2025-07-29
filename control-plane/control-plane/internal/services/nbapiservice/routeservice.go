package nbapiservice

import (
	"context"
	"fmt"
	"reflect"

	"github.com/google/uuid"
	"github.com/rs/zerolog"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
)

type RouteService struct {
	cmdHandler nodecontrol.NodeCommandHandler
}

func NewRouteService(messagingService nodecontrol.NodeCommandHandler) *RouteService {
	return &RouteService{
		cmdHandler: messagingService,
	}
}

func (s *RouteService) ListSubscriptions(
	_ context.Context,
	nodeEntry *controlplaneApi.NodeEntry,
) (*controllerapi.SubscriptionListResponse, error) {
	msg := &controllerapi.ControlMessage{
		MessageId: uuid.NewString(),
		Payload:   &controllerapi.ControlMessage_SubscriptionListRequest{},
	}

	if err := s.cmdHandler.SendMessage(nodeEntry.Id, msg); err != nil {
		return nil, fmt.Errorf("failed to send message: %w", err)
	}
	resp, err := s.cmdHandler.WaitForResponse(nodeEntry.Id,
		reflect.TypeOf(&controllerapi.ControlMessage_SubscriptionListResponse{}))
	if err != nil {
		return nil, fmt.Errorf("failed to receive SubscriptionListResponse: %w", err)
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
	return nil, fmt.Errorf("no SubscriptionListResponse received")
}

func (s *RouteService) ListConnections(
	_ context.Context,
	nodeEntry *controlplaneApi.NodeEntry,
) (*controllerapi.ConnectionListResponse, error) {
	msg := &controllerapi.ControlMessage{
		MessageId: uuid.NewString(),
		Payload:   &controllerapi.ControlMessage_ConnectionListRequest{},
	}

	if err := s.cmdHandler.SendMessage(nodeEntry.Id, msg); err != nil {
		return nil, fmt.Errorf("failed to send message: %w", err)
	}
	response, err := s.cmdHandler.WaitForResponse(nodeEntry.Id,
		reflect.TypeOf(&controllerapi.ControlMessage_ConnectionListResponse{}))
	if err != nil {
		return nil, fmt.Errorf("failed to receive ConnectionListResponse: %w", err)
	}
	if listResp := response.GetConnectionListResponse(); listResp != nil {
		for _, e := range listResp.Entries {
			fmt.Printf("id=%d %s\n", e.GetId(), e.GetConfigData())
		}
		return listResp, nil
	}
	// If we reach here, it means we didn't find a ConnectionListResponse
	return nil, fmt.Errorf("no ConnectionListResponse received")
}

func (s *RouteService) CreateConnection(
	ctx context.Context,
	nodeEntry *controlplaneApi.NodeEntry,
	connection *controllerapi.Connection,
) error {
	zlog := zerolog.Ctx(ctx)

	controllerConfigCommand := &controllerapi.ConfigurationCommand{
		ConnectionsToCreate:   []*controllerapi.Connection{connection},
		SubscriptionsToSet:    []*controllerapi.Subscription{},
		SubscriptionsToDelete: []*controllerapi.Subscription{},
	}

	messageID := uuid.NewString()
	createCommandMessage := &controllerapi.ControlMessage{
		MessageId: messageID,
		Payload: &controllerapi.ControlMessage_ConfigCommand{
			ConfigCommand: controllerConfigCommand,
		},
	}

	err := s.cmdHandler.SendMessage(nodeEntry.Id, createCommandMessage)
	if err != nil {
		return err
	}
	// wait for ACK response
	response, err := s.cmdHandler.WaitForResponse(nodeEntry.Id, reflect.TypeOf(&controllerapi.ControlMessage_Ack{}))
	if err != nil {
		return err
	}
	// check if ack is successful
	if ack := response.GetAck(); ack != nil {
		if !ack.Success {
			return fmt.Errorf("failed to create connection: %s", ack.Messages)
		}
		logAckMessage(ctx, ack)
		zlog.Debug().Msg("Connection created successfully.")
	}

	return nil
}

func (s *RouteService) CreateSubscription(
	ctx context.Context,
	nodeEntry *controlplaneApi.NodeEntry,
	subscription *controllerapi.Subscription,
) error {
	zlog := zerolog.Ctx(ctx)

	controllerConfigCommand := &controllerapi.ConfigurationCommand{
		ConnectionsToCreate:   []*controllerapi.Connection{},
		SubscriptionsToSet:    []*controllerapi.Subscription{subscription},
		SubscriptionsToDelete: []*controllerapi.Subscription{},
	}

	messageID := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageID,
		Payload: &controllerapi.ControlMessage_ConfigCommand{
			ConfigCommand: controllerConfigCommand,
		},
	}

	err := s.cmdHandler.SendMessage(nodeEntry.Id, msg)
	if err != nil {
		return err
	}
	// wait for ACK response
	response, err := s.cmdHandler.WaitForResponse(nodeEntry.Id,
		reflect.TypeOf(&controllerapi.ControlMessage_Ack{}))
	if err != nil {
		return err
	}
	// check if ack is successful
	if ack := response.GetAck(); ack != nil {
		if !ack.Success {
			return fmt.Errorf("failed to create subscription: %s", ack.Messages)
		}
		logAckMessage(ctx, ack)
		zlog.Debug().Msg("Subscription created successfully.")
	}

	return nil
}

func (s *RouteService) DeleteSubscription(
	ctx context.Context,
	nodeEntry *controlplaneApi.NodeEntry,
	subscription *controllerapi.Subscription,
) error {
	zlog := zerolog.Ctx(ctx)

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

	err := s.cmdHandler.SendMessage(nodeEntry.Id, msg)
	if err != nil {
		return err
	}
	// wait for ACK response
	response, err := s.cmdHandler.WaitForResponse(nodeEntry.Id, reflect.TypeOf(&controllerapi.ControlMessage_Ack{}))
	if err != nil {
		return err
	}
	// check if ack is successful
	if ack := response.GetAck(); ack != nil {
		if !ack.Success {
			return fmt.Errorf("failed to delete subscription: %s", ack.Messages)
		}
		logAckMessage(ctx, ack)
		zlog.Debug().Msg("Subscription deleted successfully.")
	}

	return nil
}

// logAckMessage logs the details of an ACK message received from a node.
func logAckMessage(ctx context.Context, ack *controllerapi.Ack) {
	zlog := zerolog.Ctx(ctx)
	zlog.Debug().Msgf(
		"ACK received for %s: success=%t\n",
		ack.OriginalMessageId,
		ack.Success,
	)
	if len(ack.Messages) > 0 {
		for i, ackMsg := range ack.Messages {
			zlog.Debug().Msgf("    [%d] %s\n", i+1, ackMsg)
		}
	}
}
