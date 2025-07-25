package nbapiservice

import (
	"context"
	"fmt"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
)

type GroupService struct {
	dbService db.DataAccess
}

func NewGroupService(dbService db.DataAccess) *GroupService {
	return &GroupService{
		dbService: dbService,
	}
}

func (s *GroupService) CreateChannel(
	_ context.Context,
	_ *controlplaneApi.CreateChannelRequest,
) (*controlplaneApi.CreateChannelResponse, error) {
	return nil, fmt.Errorf("not implemented")
}

func (s *GroupService) DeleteChannel(
	_ context.Context,
	_ *controlplaneApi.DeleteChannelRequest) (*controllerapi.Ack, error) {
	return nil, fmt.Errorf("not implemented")
}

func (s *GroupService) AddParticipant(
	_ context.Context,
	_ *controlplaneApi.AddParticipantRequest) (*controllerapi.Ack, error) {
	return nil, fmt.Errorf("not implemented")
}

func (s *GroupService) DeleteParticipant(
	_ context.Context,
	_ *controlplaneApi.DeleteParticipantRequest) (*controllerapi.Ack, error) {
	return nil, fmt.Errorf("not implemented")
}

func (s *GroupService) ListChannels(
	_ context.Context,
	_ *controlplaneApi.ListChannelsRequest) (*controlplaneApi.ListChannelsResponse, error) {
	return nil, fmt.Errorf("not implemented")
}

func (s *GroupService) ListParticipants(
	_ context.Context,
	_ *controlplaneApi.ListParticipantsCommand) (
	*controlplaneApi.ListParticipantsResponse, error) {
	return nil, fmt.Errorf("not implemented")
}
