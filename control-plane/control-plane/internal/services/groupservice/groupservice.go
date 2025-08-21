package groupservice

import (
	"context"
	"fmt"
	"reflect"

	"github.com/google/uuid"
	"github.com/rs/zerolog"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
	util "github.com/agntcy/slim/control-plane/control-plane/internal/util"
)

// moderator_id-random_string
const ChannelIDFormat = "%s-%s"

type GroupService struct {
	dbService  db.DataAccess
	cmdHandler nodecontrol.NodeCommandHandler
}

func NewGroupService(dbService db.DataAccess, cmdHandler nodecontrol.NodeCommandHandler) *GroupService {
	return &GroupService{
		dbService:  dbService,
		cmdHandler: cmdHandler,
	}
}

func (s *GroupService) CreateChannel(
	ctx context.Context,
	createChannelRequest *controlplaneApi.CreateChannelRequest,
	nodeEntry *controlplaneApi.NodeEntry,
) (*controlplaneApi.CreateChannelResponse, error) {
	zlog := zerolog.Ctx(ctx)

	if len(createChannelRequest.Moderators) == 0 {
		return nil, fmt.Errorf("at least one moderator is required to create a channel")
	}

	randID, err := util.GetRandomID(18)
	if err != nil {
		return nil, fmt.Errorf("failed to generate random ID: %w", err)
	}
	channelID := fmt.Sprintf(ChannelIDFormat, createChannelRequest.Moderators[0], randID)

	// Sending control message
	msg := &controllerapi.ControlMessage{
		MessageId: uuid.NewString(),
		Payload: &controllerapi.ControlMessage_CreateChannelRequest{
			CreateChannelRequest: &controllerapi.CreateChannelRequest{
				ChannelId:  channelID,
				Moderators: createChannelRequest.Moderators,
			},
		},
	}

	if sendErr := s.cmdHandler.SendMessage(nodeEntry.Id, msg); sendErr != nil {
		return nil, fmt.Errorf("failed to send message: %w", sendErr)
	}
	// wait for ACK response
	response, err := s.cmdHandler.WaitForResponse(nodeEntry.Id,
		reflect.TypeOf(&controllerapi.ControlMessage_Ack{}))
	if err != nil {
		return nil, err
	}
	// check if ack is successful
	if ack := response.GetAck(); ack != nil {
		if !ack.Success {
			return nil, fmt.Errorf("failed to create channel: %s", ack.Messages)
		}
		logAckMessage(ctx, ack)
		zlog.Debug().Msg("Channel created successfully.")
	}

	// Saving channel to DB
	err = s.dbService.SaveChannel(channelID, createChannelRequest.Moderators)
	if err != nil {
		return nil, fmt.Errorf("failed to save channel: %w", err)
	}
	zlog.Debug().Msg("Channel saved successfully.")

	return &controlplaneApi.CreateChannelResponse{
		ChannelId: channelID,
	}, nil
}

func (s *GroupService) DeleteChannel(
	ctx context.Context,
	deleteChannelRequest *controllerapi.DeleteChannelRequest,
	nodeEntry *controlplaneApi.NodeEntry,
) (*controllerapi.Ack, error) {
	zlog := zerolog.Ctx(ctx)

	if deleteChannelRequest.ChannelId == "" {
		return nil, fmt.Errorf("channel ID cannot be empty")
	}

	// Check if channel exist
	_, err := s.dbService.GetChannel(deleteChannelRequest.ChannelId)
	if err != nil {
		return nil, fmt.Errorf("failed to get channel from DB: %w", err)
	}

	// Sending control message
	msg := &controllerapi.ControlMessage{
		MessageId: uuid.NewString(),
		Payload: &controllerapi.ControlMessage_DeleteChannelRequest{
			DeleteChannelRequest: deleteChannelRequest,
		},
	}

	zlog.Debug().Msgf("Sending DeleteChannelRequest for channel %s", deleteChannelRequest.ChannelId)
	if handlerError := s.cmdHandler.SendMessage(nodeEntry.Id, msg); handlerError != nil {
		return nil, fmt.Errorf("failed to send message: %w", handlerError)
	}

	// wait for ACK response
	response, err := s.cmdHandler.WaitForResponse(nodeEntry.Id,
		reflect.TypeOf(&controllerapi.ControlMessage_Ack{}))
	if err != nil {
		return nil, err
	}
	// check if ack is successful
	if ack := response.GetAck(); ack != nil {
		if !ack.Success {
			return nil, fmt.Errorf("failed to delete participant: %s", ack.Messages)
		}
		logAckMessage(ctx, ack)
		zlog.Debug().Msg("Ack message received, channel deleted successfully.")
	}

	err = s.dbService.DeleteChannel(deleteChannelRequest.ChannelId)
	if err != nil {
		return nil, fmt.Errorf("failed to delete channel: %w", err)
	}
	zlog.Debug().Msg("Channel deleted successfully.")

	return &controllerapi.Ack{
		Success: true,
	}, nil
}

func (s *GroupService) AddParticipant(
	ctx context.Context,
	addParticipantRequest *controllerapi.AddParticipantRequest,
	nodeEntry *controlplaneApi.NodeEntry,
) (*controllerapi.Ack, error) {
	zlog := zerolog.Ctx(ctx)

	if addParticipantRequest.ChannelId == "" {
		return nil, fmt.Errorf("channel ID cannot be empty")
	}
	if addParticipantRequest.ParticipantId == "" {
		return nil, fmt.Errorf("participant ID cannot be empty")
	}

	// Sending control message
	msg := &controllerapi.ControlMessage{
		MessageId: uuid.NewString(),
		Payload: &controllerapi.ControlMessage_AddParticipantRequest{
			AddParticipantRequest: addParticipantRequest,
		},
	}

	zlog.Debug().Msgf("Sending AddParticipantRequest for channel %s", addParticipantRequest.ChannelId)
	if err := s.cmdHandler.SendMessage(nodeEntry.Id, msg); err != nil {
		return nil, fmt.Errorf("failed to send message: %w", err)
	}

	// wait for ACK response
	response, err := s.cmdHandler.WaitForResponse(nodeEntry.Id,
		reflect.TypeOf(&controllerapi.ControlMessage_Ack{}))
	if err != nil {
		return nil, err
	}
	// check if ack is successful
	if ack := response.GetAck(); ack != nil {
		if !ack.Success {
			return nil, fmt.Errorf("failed to add participant: %s", ack.Messages)
		}
		logAckMessage(ctx, ack)
		zlog.Debug().Msg("Ack message received, participant added successfully.")
	}

	// Control plane side DB operations
	channel, err := s.dbService.GetChannel(addParticipantRequest.ChannelId)
	if err != nil {
		return nil, fmt.Errorf("failed to get channel from DB: %w", err)
	}

	for _, participant := range channel.Participants {
		if participant == addParticipantRequest.ParticipantId {
			return nil, fmt.Errorf("participant %s already exists in channel %s",
				addParticipantRequest.ParticipantId, addParticipantRequest.ChannelId)
		}
	}

	channel.Participants = append(channel.Participants, addParticipantRequest.ParticipantId)
	err = s.dbService.UpdateChannel(channel)
	if err != nil {
		return nil, fmt.Errorf("failed to update channel: %w", err)
	}
	zlog.Debug().Msg("Channel updated, participant added successfully.")

	return &controllerapi.Ack{
		Success: true,
	}, nil
}

func (s *GroupService) DeleteParticipant(
	ctx context.Context,
	deleteParticipantRequest *controllerapi.DeleteParticipantRequest,
	nodeEntry *controlplaneApi.NodeEntry,
) (*controllerapi.Ack, error) {
	zlog := zerolog.Ctx(ctx)
	if deleteParticipantRequest.ChannelId == "" {
		return nil, fmt.Errorf("channel ID cannot be empty")
	}
	if deleteParticipantRequest.ParticipantId == "" {
		return nil, fmt.Errorf("participant ID cannot be empty")
	}
	channel, err := s.dbService.GetChannel(deleteParticipantRequest.ChannelId)
	if err != nil {
		return nil, fmt.Errorf("failed to get channel: %w", err)
	}
	foundIdx := -1
	for idx, participant := range channel.Participants {
		if participant == deleteParticipantRequest.ParticipantId {
			foundIdx = idx
			break
		}
	}

	if foundIdx == -1 {
		return nil, fmt.Errorf("participant %s not found in channel %s",
			deleteParticipantRequest.ParticipantId, deleteParticipantRequest.ChannelId)
	}

	// Sending control message
	msg := &controllerapi.ControlMessage{
		MessageId: uuid.NewString(),
		Payload: &controllerapi.ControlMessage_DeleteParticipantRequest{
			DeleteParticipantRequest: deleteParticipantRequest,
		},
	}

	zlog.Debug().Msgf("Sending DeleteParticipantRequest for channel %s", deleteParticipantRequest.ChannelId)
	if handlerError := s.cmdHandler.SendMessage(nodeEntry.Id, msg); handlerError != nil {
		return nil, fmt.Errorf("failed to send message: %w", handlerError)
	}

	// wait for ACK response
	response, err := s.cmdHandler.WaitForResponse(nodeEntry.Id,
		reflect.TypeOf(&controllerapi.ControlMessage_Ack{}))
	if err != nil {
		return nil, err
	}
	// check if ack is successful
	if ack := response.GetAck(); ack != nil {
		if !ack.Success {
			return nil, fmt.Errorf("failed to delete participant: %s", ack.Messages)
		}
		logAckMessage(ctx, ack)
		zlog.Debug().Msg("Ack message received, participant deleted successfully.")
	}

	// Control plane side DB operations
	channel.Participants = append(channel.Participants[:foundIdx], channel.Participants[foundIdx+1:]...)

	err = s.dbService.UpdateChannel(channel)
	if err != nil {
		return nil, fmt.Errorf("failed to update channel: %w", err)
	}
	zlog.Debug().Msg("Channel updated, participant deleted successfully.")

	return &controllerapi.Ack{
		Success: true,
	}, nil
}

func (s *GroupService) ListChannels(
	ctx context.Context,
	_ *controllerapi.ListChannelsRequest) (*controllerapi.ListChannelsResponse, error) {
	zlog := zerolog.Ctx(ctx)
	channels, err := s.dbService.ListChannels()
	if err != nil {
		return nil, fmt.Errorf("failed to list channels: %w", err)
	}

	channelIDs := make([]string, 0, len(channels))
	for _, channel := range channels {
		channelIDs = append(channelIDs, channel.ID)
	}
	zlog.Debug().Msg("Channels listed successfully.")

	return &controllerapi.ListChannelsResponse{
		ChannelId: channelIDs,
	}, nil
}

func (s *GroupService) ListParticipants(
	ctx context.Context,
	listParticipantsRequest *controllerapi.ListParticipantsRequest) (
	*controllerapi.ListParticipantsResponse, error) {
	zlog := zerolog.Ctx(ctx)

	if listParticipantsRequest.ChannelId == "" {
		return nil, fmt.Errorf("channel ID cannot be empty")
	}

	channel, err := s.dbService.GetChannel(listParticipantsRequest.ChannelId)
	if err != nil {
		return nil, fmt.Errorf("failed to get channel: %w", err)
	}
	zlog.Debug().Msg("Participants listed successfully.")

	return &controllerapi.ListParticipantsResponse{
		ParticipantId: channel.Participants,
	}, nil
}

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
