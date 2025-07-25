package nbapiservice

import (
	"context"
	"fmt"

	"github.com/rs/zerolog"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	util "github.com/agntcy/slim/control-plane/control-plane/internal/util"
)

// moderator_id-random_string
const ChannelIDFormat = "%s-%s"

type GroupService struct {
	dbService db.DataAccess
}

func NewGroupService(dbService db.DataAccess) *GroupService {
	return &GroupService{
		dbService: dbService,
	}
}

func (s *GroupService) CreateChannel(
	ctx context.Context,
	createChannelRequest *controlplaneApi.CreateChannelRequest,
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
	err = s.dbService.SaveChannel(channelID, createChannelRequest.Moderators)
	if err != nil {
		return nil, fmt.Errorf("failed to save channel: %w", err)
	}
	zlog.Debug().Msg("Channel created successfully.")

	return &controlplaneApi.CreateChannelResponse{
		ChannelId: channelID,
	}, nil
}

func (s *GroupService) DeleteChannel(
	ctx context.Context,
	deleteChannelRequest *controlplaneApi.DeleteChannelRequest) (*controllerapi.Ack, error) {
	zlog := zerolog.Ctx(ctx)

	if deleteChannelRequest.ChannelId == "" {
		return nil, fmt.Errorf("channel ID cannot be empty")
	}

	err := s.dbService.DeleteChannel(deleteChannelRequest.ChannelId)
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
	addParticipantRequest *controlplaneApi.AddParticipantRequest) (*controllerapi.Ack, error) {
	zlog := zerolog.Ctx(ctx)

	if addParticipantRequest.ChannelId == "" {
		return nil, fmt.Errorf("channel ID cannot be empty")
	}
	if addParticipantRequest.ParticipantId == "" {
		return nil, fmt.Errorf("participant ID cannot be empty")
	}

	channel, err := s.dbService.GetChannel(addParticipantRequest.ChannelId)
	if err != nil {
		return nil, fmt.Errorf("failed to get channel: %w", err)
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
	zlog.Debug().Msg("Participant added successfully.")

	return &controllerapi.Ack{
		Success: true,
	}, nil
}

func (s *GroupService) DeleteParticipant(
	ctx context.Context,
	deleteParticipantRequest *controlplaneApi.DeleteParticipantRequest) (*controllerapi.Ack, error) {
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

	channel.Participants = append(channel.Participants[:foundIdx], channel.Participants[foundIdx+1:]...)

	err = s.dbService.UpdateChannel(channel)
	if err != nil {
		return nil, fmt.Errorf("failed to update channel: %w", err)
	}
	zlog.Debug().Msg("Participant deleted successfully.")

	return &controllerapi.Ack{
		Success: true,
	}, nil
}

func (s *GroupService) ListChannels(
	ctx context.Context,
	_ *controlplaneApi.ListChannelsRequest) (*controlplaneApi.ListChannelsResponse, error) {
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

	return &controlplaneApi.ListChannelsResponse{
		ChannelId: channelIDs,
	}, nil
}

func (s *GroupService) ListParticipants(
	ctx context.Context,
	listParticipantsRequest *controlplaneApi.ListParticipantsRequest) (
	*controlplaneApi.ListParticipantsResponse, error) {
	zlog := zerolog.Ctx(ctx)

	if listParticipantsRequest.ChannelId == "" {
		return nil, fmt.Errorf("channel ID cannot be empty")
	}

	channel, err := s.dbService.GetChannel(listParticipantsRequest.ChannelId)
	if err != nil {
		return nil, fmt.Errorf("failed to get channel: %w", err)
	}
	zlog.Debug().Msg("Participants listed successfully.")

	return &controlplaneApi.ListParticipantsResponse{
		ParticipantId: channel.Participants,
	}, nil
}
