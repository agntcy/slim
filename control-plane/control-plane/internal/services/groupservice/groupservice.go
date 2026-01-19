package groupservice

import (
	"context"
	"fmt"
	"reflect"
	"slices"

	"github.com/google/uuid"
	"github.com/rs/zerolog"
	"google.golang.org/protobuf/types/known/wrapperspb"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	commonutil "github.com/agntcy/slim/control-plane/common/util"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
	util "github.com/agntcy/slim/control-plane/control-plane/internal/util"
)

// moderator_id-random_string
const ChannelNameFormat = "%s/%s/%s"

type GroupManager interface {
	CreateChannel(
		ctx context.Context,
		createChannelRequest *controlplaneApi.CreateChannelRequest,
	) (*controlplaneApi.CreateChannelResponse, error)
	DeleteChannel(
		ctx context.Context,
		deleteChannelRequest *controllerapi.DeleteChannelRequest,
	) (*controllerapi.Ack, error)
	AddParticipant(
		ctx context.Context,
		addParticipantRequest *controllerapi.AddParticipantRequest,
	) (*controllerapi.Ack, error)
	DeleteParticipant(
		ctx context.Context,
		deleteParticipantRequest *controllerapi.DeleteParticipantRequest,
	) (*controllerapi.Ack, error)
	ListChannels(
		ctx context.Context,
		listChannelsRequest *controllerapi.ListChannelsRequest,
	) (*controllerapi.ListChannelsResponse, error)
	ListParticipants(
		ctx context.Context,
		listParticipantsRequest *controllerapi.ListParticipantsRequest,
	) (*controllerapi.ListParticipantsResponse, error)
	GetChannelDetails(
		ctx context.Context,
		channelID string,
	) (db.Channel, error)
}

type DataAccess interface {
	SaveChannel(channelID string, moderators []string) error
	DeleteChannel(channelID string) error
	GetChannel(channelID string) (db.Channel, error)
	UpdateChannel(channel db.Channel) error
	ListChannels() ([]db.Channel, error)
	GetDestinationNodeIDForName(Component0 string, Component1 string, Component2 string,
		ComponentID *wrapperspb.UInt64Value) string
}

type GroupService struct {
	dbService  DataAccess
	cmdHandler nodecontrol.NodeCommandHandler
}

func NewGroupService(dbService DataAccess, cmdHandler nodecontrol.NodeCommandHandler) GroupManager {
	return &GroupService{
		dbService:  dbService,
		cmdHandler: cmdHandler,
	}
}

func (s *GroupService) CreateChannel(
	ctx context.Context,
	createChannelRequest *controlplaneApi.CreateChannelRequest,
) (*controlplaneApi.CreateChannelResponse, error) {
	zlog := zerolog.Ctx(ctx)

	zlog.Debug().Msgf("Received CreateChannelRequest: %+v", createChannelRequest)

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

	// Find the node where the moderator is located
	nodeID, err := s.getModeratorNode(ctx, createChannelRequest.Moderators)
	if err != nil {
		return nil, fmt.Errorf("failed to find moderator node: %w", err)
	}

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
	zlog.Debug().Msgf("Sending CreateChannelRequest for channel %s to node: %s", channelName, nodeID)
	if sendErr := s.cmdHandler.SendMessage(ctx, nodeID, msg); sendErr != nil {
		return nil, fmt.Errorf("failed to send message: %w", sendErr)
	}
	// wait for ACK response
	response, err := s.cmdHandler.WaitForResponse(
		ctx,
		nodeID,
		reflect.TypeOf(&controllerapi.ControlMessage_Ack{}),
		messageID,
	)
	if err != nil {
		return nil, err
	}
	// check if ack is successful
	ack := response.GetAck()
	if ack == nil {
		zlog.Debug().Msg("failed to create channel: ack is nil")
		return nil, fmt.Errorf("failed to create channel")
	}
	logAckMessage(ctx, ack)
	zlog.Debug().Msgf("Channel creation result for %s: success=%t", channelName, ack.Success)
	if !ack.Success {
		return nil, fmt.Errorf("failed to create channel: %s", ack.Messages)
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

	// Find the node where the moderator is located
	nodeID, err := s.getModeratorNode(ctx, channel.Moderators)
	if err != nil {
		return nil, fmt.Errorf("failed to find moderator node: %w", err)
	}

	// Sending control message
	messageID := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageID,
		Payload: &controllerapi.ControlMessage_DeleteChannelRequest{
			DeleteChannelRequest: deleteChannelRequest,
		},
	}

	zlog.Debug().Msgf("Sending DeleteChannelRequest for channel %s to node: %s",
		deleteChannelRequest.ChannelName, nodeID)
	if handlerError := s.cmdHandler.SendMessage(ctx, nodeID, msg); handlerError != nil {
		return nil, fmt.Errorf("failed to send message: %w", handlerError)
	}

	// wait for ACK response
	response, err := s.cmdHandler.WaitForResponse(
		ctx,
		nodeID,
		reflect.TypeOf(&controllerapi.ControlMessage_Ack{}),
		messageID,
	)
	if err != nil {
		return nil, err
	}
	// check if ack is successful
	ack := response.GetAck()
	if ack == nil {
		zlog.Debug().Msg("failed to delete channel: ack is nil")
		return nil, fmt.Errorf("failed to delete channel")
	}
	logAckMessage(ctx, ack)
	zlog.Debug().Msgf("Channel deletion result for %s: success=%t", deleteChannelRequest.ChannelName, ack.Success)
	if !ack.Success {
		return ack, nil
	}

	err = s.dbService.DeleteChannel(deleteChannelRequest.ChannelName)
	if err != nil {
		return nil, fmt.Errorf("failed to delete channel: %w", err)
	}
	zlog.Debug().Msg("Channel deleted successfully.")

	return ack, nil
}

func (s *GroupService) AddParticipant(
	ctx context.Context,
	addParticipantRequest *controllerapi.AddParticipantRequest,
) (*controllerapi.Ack, error) {
	zlog := zerolog.Ctx(ctx)

	_, err := commonutil.ValidateName(addParticipantRequest.ChannelName, 3)
	if err != nil {
		return nil, fmt.Errorf("invalid channel name: %w", err)
	}

	_, err = commonutil.ValidateName(addParticipantRequest.ParticipantName, 3)
	if err != nil {
		return nil, fmt.Errorf("invalid participant name: %w", err)
	}

	// Control plane side DB operations
	channel, err := s.dbService.GetChannel(addParticipantRequest.ChannelName)
	if err != nil {
		return nil, fmt.Errorf("failed to get channel from DB: %w", err)
	}

	addParticipantRequest.Moderators = channel.Moderators

	// Find the node where the moderator is located
	nodeID, err := s.getModeratorNode(ctx, channel.Moderators)
	if err != nil {
		return nil, fmt.Errorf("failed to find moderator node: %w", err)
	}

	// Sending control message
	messageID := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageID,
		Payload: &controllerapi.ControlMessage_AddParticipantRequest{
			AddParticipantRequest: addParticipantRequest,
		},
	}

	zlog.Debug().Msgf("Sending AddParticipantRequest for channel %s to node: %s",
		addParticipantRequest.ChannelName, nodeID)
	if err = s.cmdHandler.SendMessage(ctx, nodeID, msg); err != nil {
		return nil, fmt.Errorf("failed to send message: %w", err)
	}

	// wait for ACK response
	response, err := s.cmdHandler.WaitForResponse(
		ctx,
		nodeID,
		reflect.TypeOf(&controllerapi.ControlMessage_Ack{}),
		messageID,
	)
	if err != nil {
		return nil, err
	}
	// check if ack is successful
	ack := response.GetAck()
	if ack == nil {
		zlog.Debug().Msg("failed to add participant: ack is nil")
		return nil, fmt.Errorf("failed to add participant")
	}
	logAckMessage(ctx, ack)
	zlog.Debug().Msgf("AddParticipant result success=%t", ack.Success)
	if !ack.Success {
		return ack, nil
	}

	if slices.Contains(channel.Participants, addParticipantRequest.ParticipantName) {
		return nil, fmt.Errorf("participant %s already exists in channel %s",
			addParticipantRequest.ParticipantName, addParticipantRequest.ChannelName)
	}

	channel.Participants = append(channel.Participants, addParticipantRequest.ParticipantName)
	err = s.dbService.UpdateChannel(channel)
	if err != nil {
		return nil, fmt.Errorf("failed to update channel: %w", err)
	}
	zlog.Debug().Msg("Channel updated, participant added successfully.")

	return ack, nil
}

func (s *GroupService) DeleteParticipant(
	ctx context.Context,
	deleteParticipantRequest *controllerapi.DeleteParticipantRequest,
) (*controllerapi.Ack, error) {
	zlog := zerolog.Ctx(ctx)
	if deleteParticipantRequest.ChannelName == "" {
		return nil, fmt.Errorf("channel ID cannot be empty")
	}
	if deleteParticipantRequest.ParticipantName == "" {
		return nil, fmt.Errorf("participant ID cannot be empty")
	}
	channel, err := s.dbService.GetChannel(deleteParticipantRequest.ChannelName)
	if err != nil {
		return nil, fmt.Errorf("failed to get channel: %w", err)
	}
	foundIdx := -1
	for idx, participant := range channel.Participants {
		if participant == deleteParticipantRequest.ParticipantName {
			foundIdx = idx
			break
		}
	}

	if foundIdx == -1 {
		return nil, fmt.Errorf("participant %s not found in channel %s",
			deleteParticipantRequest.ParticipantName, deleteParticipantRequest.ChannelName)
	}

	deleteParticipantRequest.Moderators = channel.Moderators

	// Find the node where the moderator is located
	nodeID, err := s.getModeratorNode(ctx, channel.Moderators)
	if err != nil {
		return nil, fmt.Errorf("failed to find moderator node: %w", err)
	}

	// Sending control message
	messageID := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageID,
		Payload: &controllerapi.ControlMessage_DeleteParticipantRequest{
			DeleteParticipantRequest: deleteParticipantRequest,
		},
	}

	zlog.Debug().Msgf("Sending DeleteParticipantRequest for channel %s to node: %s",
		deleteParticipantRequest.ChannelName, nodeID)
	if handlerError := s.cmdHandler.SendMessage(ctx, nodeID, msg); handlerError != nil {
		return nil, fmt.Errorf("failed to send message: %w", handlerError)
	}

	// wait for ACK response
	response, err := s.cmdHandler.WaitForResponse(
		ctx,
		nodeID,
		reflect.TypeOf(&controllerapi.ControlMessage_Ack{}),
		messageID,
	)
	if err != nil {
		return nil, err
	}
	// check if ack is successful
	ack := response.GetAck()
	if ack == nil {
		zlog.Debug().Msg("failed to delete participant: ack is nil")
		return nil, fmt.Errorf("failed to delete participant")
	}
	logAckMessage(ctx, ack)
	zlog.Debug().Msgf("DeleteParticipant result success=%t", ack.Success)
	if !ack.Success {
		return ack, nil
	}
	// Control plane side DB operations
	channel.Participants = append(channel.Participants[:foundIdx], channel.Participants[foundIdx+1:]...)

	err = s.dbService.UpdateChannel(channel)
	if err != nil {
		return nil, fmt.Errorf("failed to update channel: %w", err)
	}
	zlog.Debug().Msg("Channel updated, participant deleted successfully.")

	return ack, nil
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
		ParticipantName: channel.Participants,
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

// getModeratorNode returns the node ID where the moderator is located
func (s *GroupService) getModeratorNode(_ context.Context, moderators []string) (string, error) {
	if len(moderators) == 0 {
		return "", fmt.Errorf("no moderators provided")
	}

	moderatorToFind := moderators[0]
	parts, err := commonutil.ValidateName(moderatorToFind, 4)
	if err != nil {
		return "", fmt.Errorf("failed to parse moderator route: %w", err)
	}

	organization := parts[0]
	namespace := parts[1]
	agentType := parts[2]
	var appInstance *wrapperspb.UInt64Value
	if len(parts) > 3 && parts[3] != "" {
		// Parse the appInstance as uint64
		var instanceID uint64
		_, parseErr := fmt.Sscanf(parts[3], "%d", &instanceID)
		if parseErr == nil {
			appInstance = wrapperspb.UInt64(instanceID)
		}
	}

	// Use the new method to find the node where the moderator is located
	destNodeID := s.dbService.GetDestinationNodeIDForName(organization, namespace, agentType, appInstance)
	if destNodeID == "" {
		return "", fmt.Errorf("no node found for moderator %s", moderatorToFind)
	}

	return destNodeID, nil
}
