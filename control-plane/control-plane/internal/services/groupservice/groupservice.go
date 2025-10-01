package groupservice

import (
	"context"
	"fmt"
	"reflect"
	"slices"

	"github.com/google/uuid"
	"github.com/rs/zerolog"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	commonutil "github.com/agntcy/slim/control-plane/common/util"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
	util "github.com/agntcy/slim/control-plane/control-plane/internal/util"
)

// moderator_id-random_string
const ChannelNameFormat = "%s/%s/%s"

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

	zlog.Error().Msgf("Received CreateChannelRequest: %+v", createChannelRequest)

	if len(createChannelRequest.Moderators) == 0 {
		return nil, fmt.Errorf("at least one moderator is required to create a channel")
	}

	// For now we will just use the first moderator to determine the channel name
	// TODO: enhance this logic to support multiple moderators
	moderatorName := createChannelRequest.Moderators[0]

	// make sure the moderator name is well-formed
	parts, err := commonutil.ValidateName(moderatorName, 4)
	if err != nil {
		return nil, fmt.Errorf("invalid moderator name: %w", err)
	}

	randID, err := util.GetRandomID(18)
	if err != nil {
		return nil, fmt.Errorf("failed to generate random ID: %w", err)
	}
	channelName := fmt.Sprintf(ChannelNameFormat, parts[0], parts[1], randID)

	zlog.Debug().Msgf("Generated channel name: %s", channelName)

	// Sending control message
	messageID := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageID,
		Payload: &controllerapi.ControlMessage_CreateChannelRequest{
			CreateChannelRequest: &controllerapi.CreateChannelRequest{
				ChannelName: channelName,
				Moderators:  createChannelRequest.Moderators,
			},
		},
	}
	zlog.Debug().Msgf("Sending CreateChannelRequest for channel %s to node: %s", channelName, nodeEntry.Id)
	if sendErr := s.cmdHandler.SendMessage(ctx, nodeEntry.Id, msg); sendErr != nil {
		return nil, fmt.Errorf("failed to send message: %w", sendErr)
	}
	// wait for ACK response
	response, err := s.cmdHandler.WaitForResponse(
		ctx,
		nodeEntry.Id,
		reflect.TypeOf(&controllerapi.ControlMessage_Ack{}),
		messageID,
	)
	if err != nil {
		return nil, err
	}
	// check if ack is successful
	if ack := response.GetAck(); ack != nil {
		if !ack.Success {
			return nil, fmt.Errorf("failed to create channel: %s", ack.Messages)
		}
		logAckMessage(ctx, ack)
		zlog.Debug().Msgf("Channel created successfully: %s", channelName)
	}

	// Saving channel to DB
	err = s.dbService.SaveChannel(channelName, createChannelRequest.Moderators)
	if err != nil {
		return nil, fmt.Errorf("failed to save channel: %w", err)
	}
	zlog.Debug().Msg("Channel saved successfully.")

	return &controlplaneApi.CreateChannelResponse{
		ChannelName: channelName,
	}, nil
}

func (s *GroupService) DeleteChannel(
	ctx context.Context,
	deleteChannelRequest *controllerapi.DeleteChannelRequest,
	nodeEntry *controlplaneApi.NodeEntry,
) (*controllerapi.Ack, error) {
	zlog := zerolog.Ctx(ctx)

	_, err := commonutil.ValidateName(deleteChannelRequest.ChannelName, 3)
	if err != nil {
		return nil, fmt.Errorf("invalid channel name: %w", err)
	}

	// Check if channel exist
	channel, err := s.dbService.GetChannel(deleteChannelRequest.ChannelName)
	if err != nil {
		return nil, fmt.Errorf("failed to get channel from DB: %w", err)
	}

	deleteChannelRequest.Moderators = channel.Moderators

	// Sending control message
	messageID := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageID,
		Payload: &controllerapi.ControlMessage_DeleteChannelRequest{
			DeleteChannelRequest: deleteChannelRequest,
		},
	}

	zlog.Debug().Msgf("Sending DeleteChannelRequest for channel %s to node: %s",
		deleteChannelRequest.ChannelName, nodeEntry.Id)
	if handlerError := s.cmdHandler.SendMessage(ctx, nodeEntry.Id, msg); handlerError != nil {
		return nil, fmt.Errorf("failed to send message: %w", handlerError)
	}

	// wait for ACK response
	response, err := s.cmdHandler.WaitForResponse(
		ctx,
		nodeEntry.Id,
		reflect.TypeOf(&controllerapi.ControlMessage_Ack{}),
		messageID,
	)
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

	err = s.dbService.DeleteChannel(deleteChannelRequest.ChannelName)
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

	_, err := commonutil.ValidateName(addParticipantRequest.ChannelName, 3)
	if err != nil {
		return nil, fmt.Errorf("invalid channel name: %w", err)
	}

	_, err = commonutil.ValidateName(addParticipantRequest.ParticipantId, 4)
	if err != nil {
		return nil, fmt.Errorf("invalid participant name: %w", err)
	}

	// Control plane side DB operations
	channel, err := s.dbService.GetChannel(addParticipantRequest.ChannelName)
	if err != nil {
		return nil, fmt.Errorf("failed to get channel from DB: %w", err)
	}

	addParticipantRequest.Moderators = channel.Moderators

	// Sending control message
	messageID := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageID,
		Payload: &controllerapi.ControlMessage_AddParticipantRequest{
			AddParticipantRequest: addParticipantRequest,
		},
	}

	zlog.Debug().Msgf("Sending AddParticipantRequest for channel %s to node: %s",
		addParticipantRequest.ChannelName, nodeEntry.Id)
	if err = s.cmdHandler.SendMessage(ctx, nodeEntry.Id, msg); err != nil {
		return nil, fmt.Errorf("failed to send message: %w", err)
	}

	// wait for ACK response
	response, err := s.cmdHandler.WaitForResponse(
		ctx,
		nodeEntry.Id,
		reflect.TypeOf(&controllerapi.ControlMessage_Ack{}),
		messageID,
	)
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

	if slices.Contains(channel.Participants, addParticipantRequest.ParticipantId) {
		return nil, fmt.Errorf("participant %s already exists in channel %s",
			addParticipantRequest.ParticipantId, addParticipantRequest.ChannelName)
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
	if deleteParticipantRequest.ChannelName == "" {
		return nil, fmt.Errorf("channel ID cannot be empty")
	}
	if deleteParticipantRequest.ParticipantId == "" {
		return nil, fmt.Errorf("participant ID cannot be empty")
	}
	channel, err := s.dbService.GetChannel(deleteParticipantRequest.ChannelName)
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
			deleteParticipantRequest.ParticipantId, deleteParticipantRequest.ChannelName)
	}

	deleteParticipantRequest.Moderators = channel.Moderators

	// Sending control message
	messageID := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageID,
		Payload: &controllerapi.ControlMessage_DeleteParticipantRequest{
			DeleteParticipantRequest: deleteParticipantRequest,
		},
	}

	zlog.Debug().Msgf("Sending DeleteParticipantRequest for channel %s to node: %s",
		deleteParticipantRequest.ChannelName, nodeEntry.Id)
	if handlerError := s.cmdHandler.SendMessage(ctx, nodeEntry.Id, msg); handlerError != nil {
		return nil, fmt.Errorf("failed to send message: %w", handlerError)
	}

	// wait for ACK response
	response, err := s.cmdHandler.WaitForResponse(
		ctx,
		nodeEntry.Id,
		reflect.TypeOf(&controllerapi.ControlMessage_Ack{}),
		messageID,
	)
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

	channelNames := make([]string, 0, len(channels))
	for _, channel := range channels {
		channelNames = append(channelNames, channel.ID)
	}
	zlog.Debug().Msg("Channels listed successfully.")

	return &controllerapi.ListChannelsResponse{
		ChannelName: channelNames,
	}, nil
}

func (s *GroupService) GetChannelDetails(
	ctx context.Context,
	channelID string) (
	db.Channel, error) {
	zlog := zerolog.Ctx(ctx)

	storedChannel, err := s.dbService.GetChannel(channelID)
	if err != nil {
		return db.Channel{}, fmt.Errorf("failed to get channel: %w", err)
	}
	zlog.Debug().Msg("Channel details retrieved successfully.")

	return storedChannel, nil
}

func (s *GroupService) ListParticipants(
	ctx context.Context,
	listParticipantsRequest *controllerapi.ListParticipantsRequest) (
	*controllerapi.ListParticipantsResponse, error) {
	zlog := zerolog.Ctx(ctx)

	if listParticipantsRequest.ChannelName == "" {
		return nil, fmt.Errorf("channel ID cannot be empty")
	}

	channel, err := s.dbService.GetChannel(listParticipantsRequest.ChannelName)
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
