// groupservice_test.go - clean, valid tests for groupservice
package groupservice

import (
	"context"
	"errors"
	"reflect"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"google.golang.org/protobuf/types/known/wrapperspb"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
)

const (
	MockResponseSuccess = iota
	MockResponseFailure
	MockResponseError
)

type mockNodeCommandHandler struct {
	responseType int
}

func (m *mockNodeCommandHandler) SetResponseType(responseType int) {
	m.responseType = responseType
}

func (m *mockNodeCommandHandler) SendMessage(
	_ context.Context, _ string, _ *controllerapi.ControlMessage) error {
	return nil
}

func (m *mockNodeCommandHandler) AddStream(
	_ context.Context, _ string, _ controllerapi.ControllerService_OpenControlChannelServer) {
}

func (m *mockNodeCommandHandler) RemoveStream(_ context.Context, _ string) error {
	return nil
}

func (m *mockNodeCommandHandler) GetConnectionStatus(
	_ context.Context, _ string) (nodecontrol.NodeStatus, error) {
	return nodecontrol.NodeStatusUnknown, nil
}

func (m *mockNodeCommandHandler) UpdateConnectionStatus(
	_ context.Context, _ string, _ nodecontrol.NodeStatus) {
}

func (m *mockNodeCommandHandler) WaitForResponse(
	_ context.Context, _ string, _ reflect.Type, _ string,
) (*controllerapi.ControlMessage, error) {
	switch m.responseType {
	case MockResponseSuccess:
		return &controllerapi.ControlMessage{
			Payload: &controllerapi.ControlMessage_Ack{
				Ack: &controllerapi.Ack{
					Success: true,
				},
			},
		}, nil
	case MockResponseFailure:
		return &controllerapi.ControlMessage{
			Payload: &controllerapi.ControlMessage_Ack{
				Ack: &controllerapi.Ack{
					Success: false,
				},
			},
		}, nil
	case MockResponseError:
		return nil, errors.New("communication error")
	default:
		return nil, errors.New("unknown response type")
	}
}

func (m *mockNodeCommandHandler) WaitForResponseWithTimeout(
	_ context.Context, _ string, _ reflect.Type, _ string, _ time.Duration,
) (*controllerapi.ControlMessage, error) {
	return nil, nil
}

func (m *mockNodeCommandHandler) ResponseReceived(
	_ context.Context, _ string, _ *controllerapi.ControlMessage) {
}

// TestCreateChannel_Success verifies that creating a channel with moderators succeeds.
func TestCreateChannel_Success(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseSuccess}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()

	// Add a route to the DB so GetDestinationNodeIDForName can find the node
	route := db.Route{
		SourceNodeID: db.AllNodesID,
		DestNodeID:   "node123",
		Component0:   "org",
		Component1:   "ns",
		Component2:   "mod",
		ComponentID:  wrapperspb.UInt64(0),
		LastUpdated:  time.Now(),
	}
	_, err := dbService.AddRoute(route)
	require.NoError(t, err)

	// create a valid request
	request := &controlplaneApi.CreateChannelRequest{
		Moderators: []string{"org/ns/mod/0"},
	}
	resp, err := svc.CreateChannel(ctx, request)
	assert.NoError(t, err)
	assert.NotEmpty(t, resp.ChannelName)
	assert.Contains(t, resp.ChannelName, "org/ns/")
}

func TestCreateChannel_InvalidModeratorName(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseSuccess}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()

	// create a valid request
	request := &controlplaneApi.CreateChannelRequest{
		Moderators: []string{"asd"},
	}
	resp, err := svc.CreateChannel(ctx, request)
	assert.Error(t, err)
	assert.Nil(t, resp)
	assert.Contains(t, err.Error(), "invalid moderator name")
}

// TestCreateChannel_NoModerators verifies that missing moderators yields an error.
func TestCreateChannel_NoModerators(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseSuccess}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()
	request := &controlplaneApi.CreateChannelRequest{}
	_, err := svc.CreateChannel(ctx, request)
	assert.Error(t, err)
}

// TestCreateChannel_NoNodeFound verifies that error is returned when no node is found.
func TestCreateChannel_NoNodeFound(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseSuccess}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()
	request := &controlplaneApi.CreateChannelRequest{
		Moderators: []string{"org/ns/mod1/0"},
	}
	_, err := svc.CreateChannel(ctx, request)
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "no node found")
}

// TestDeleteChannel_Success verifies successful channel deletion.
func TestDeleteChannel_Success(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseSuccess}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()

	// Add a route to the DB so GetDestinationNodeIDForName can find the node
	route := db.Route{
		SourceNodeID: db.AllNodesID,
		DestNodeID:   "node123",
		Component0:   "org",
		Component1:   "ns",
		Component2:   "mod1",
		ComponentID:  wrapperspb.UInt64(0),
		LastUpdated:  time.Now(),
	}
	_, err := dbService.AddRoute(route)
	require.NoError(t, err)

	// Save a channel first
	err = dbService.SaveChannel("org/ns/channel123", []string{"org/ns/mod1/0"})
	require.NoError(t, err)

	request := &controllerapi.DeleteChannelRequest{
		ChannelName: "org/ns/channel123",
	}
	resp, err := svc.DeleteChannel(ctx, request)
	assert.NoError(t, err)
	assert.True(t, resp.Success)
}

// TestDeleteChannel_EmptyChannelName verifies error when channel ID is empty.
func TestDeleteChannel_EmptyChannelName(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseSuccess}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.DeleteChannelRequest{
		ChannelName: "",
	}
	_, err := svc.DeleteChannel(ctx, request)
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "name cannot be empty")
}

// TestDeleteChannel_AckFailure verifies that ACK failure is handled correctly.
func TestDeleteChannel_AckFailure(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseFailure}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()

	// Add a route to the DB so GetDestinationNodeIDForName can find the node
	route := db.Route{
		SourceNodeID: db.AllNodesID,
		DestNodeID:   "node123",
		Component0:   "org",
		Component1:   "ns",
		Component2:   "mod1",
		ComponentID:  wrapperspb.UInt64(0),
		LastUpdated:  time.Now(),
	}
	_, err := dbService.AddRoute(route)
	require.NoError(t, err)

	// Save a channel first
	err = dbService.SaveChannel("org/ns/channel123", []string{"org/ns/mod1/0"})
	require.NoError(t, err)

	request := &controllerapi.DeleteChannelRequest{
		ChannelName: "org/ns/channel123",
	}
	resp, err := svc.DeleteChannel(ctx, request)
	assert.NoError(t, err)
	assert.False(t, resp.Success)

	// Channel should still exist in DB since deletion failed
	_, getErr := dbService.GetChannel("org/ns/channel123")
	assert.NoError(t, getErr)
}

// TestAddParticipant_Success verifies successful participant addition.
func TestAddParticipant_Success(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseSuccess}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()

	// Add a route to the DB so GetDestinationNodeIDForName can find the node
	route := db.Route{
		SourceNodeID: db.AllNodesID,
		DestNodeID:   "node123",
		Component0:   "org",
		Component1:   "ns",
		Component2:   "mod1",
		ComponentID:  wrapperspb.UInt64(0),
		LastUpdated:  time.Now(),
	}
	_, err := dbService.AddRoute(route)
	require.NoError(t, err)

	// Save a channel first
	err = dbService.SaveChannel("org/ns/channel123", []string{"org/ns/mod1/0"})
	require.NoError(t, err)

	request := &controllerapi.AddParticipantRequest{
		ChannelName:     "org/ns/channel123",
		ParticipantName: "org/ns/participant123",
	}
	resp, err := svc.AddParticipant(ctx, request)
	assert.NoError(t, err)
	assert.True(t, resp.Success)
}

// TestAddParticipant_EmptyChannelName verifies error when channel ID is empty.
func TestAddParticipant_EmptyChannelName(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseSuccess}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.AddParticipantRequest{
		ChannelName:     "",
		ParticipantName: "participant123",
	}
	_, err := svc.AddParticipant(ctx, request)
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "name cannot be empty")
}

// TestAddParticipant_EmptyParticipantName verifies error when participant ID is empty.
func TestAddParticipant_EmptyParticipantName(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseSuccess}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.AddParticipantRequest{
		ChannelName:     "org/name/channel123",
		ParticipantName: "",
	}
	_, err := svc.AddParticipant(ctx, request)
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "name cannot be empty")
}

// TestAddParticipant_AckFailure verifies that ACK failure is handled correctly.
func TestAddParticipant_AckFailure(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseFailure}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()

	// Add a route to the DB so GetDestinationNodeIDForName can find the node
	route := db.Route{
		SourceNodeID: db.AllNodesID,
		DestNodeID:   "node123",
		Component0:   "org",
		Component1:   "ns",
		Component2:   "mod1",
		ComponentID:  wrapperspb.UInt64(0),
		LastUpdated:  time.Now(),
	}
	_, err := dbService.AddRoute(route)
	require.NoError(t, err)

	// Save a channel first
	err = dbService.SaveChannel("org/ns/channel123", []string{"org/ns/mod1/0"})
	require.NoError(t, err)

	request := &controllerapi.AddParticipantRequest{
		ChannelName:     "org/ns/channel123",
		ParticipantName: "org/ns/participant123",
	}
	resp, err := svc.AddParticipant(ctx, request)
	assert.NoError(t, err)
	assert.False(t, resp.Success)

	// Participant should not be added to channel since operation failed
	channel, getErr := dbService.GetChannel("org/ns/channel123")
	assert.NoError(t, getErr)
	assert.Empty(t, channel.Participants)
}

// TestDeleteParticipant_Success verifies successful participant deletion.
func TestDeleteParticipant_Success(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseSuccess}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()

	// Add a route to the DB so GetDestinationNodeIDForName can find the node
	route := db.Route{
		SourceNodeID: db.AllNodesID,
		DestNodeID:   "node123",
		Component0:   "org",
		Component1:   "ns",
		Component2:   "mod1",
		ComponentID:  wrapperspb.UInt64(0),
		LastUpdated:  time.Now(),
	}
	_, err := dbService.AddRoute(route)
	require.NoError(t, err)

	// Save a channel with participants
	err = dbService.SaveChannel("deleteParticipantChannel", []string{"org/ns/mod1/0"})
	require.NoError(t, err)

	channel, err := dbService.GetChannel("deleteParticipantChannel")
	require.NoError(t, err)
	channel.Participants = []string{"org/ns/participantToDelete/0", "org/ns/participant2/0"}
	err = dbService.UpdateChannel(channel)
	require.NoError(t, err)

	request := &controllerapi.DeleteParticipantRequest{
		ChannelName:     "deleteParticipantChannel",
		ParticipantName: "org/ns/participantToDelete/0",
	}
	resp, err := svc.DeleteParticipant(ctx, request)
	assert.NoError(t, err)
	assert.True(t, resp.Success)
}

// TestDeleteParticipant_EmptyChannelName verifies error when channel ID is empty.
func TestDeleteParticipant_EmptyChannelName(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseSuccess}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.DeleteParticipantRequest{
		ChannelName:     "",
		ParticipantName: "participant123",
	}
	_, err := svc.DeleteParticipant(ctx, request)
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "channel ID cannot be empty")
}

// TestDeleteParticipant_EmptyParticipantName verifies error when participant ID is empty.
func TestDeleteParticipant_EmptyParticipantName(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseSuccess}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.DeleteParticipantRequest{
		ChannelName:     "channel123",
		ParticipantName: "",
	}
	_, err := svc.DeleteParticipant(ctx, request)
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "participant ID cannot be empty")
}

// TestListChannels_Success verifies successful channel listing.
func TestListChannels_Success(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseSuccess}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.ListChannelsRequest{}
	resp, err := svc.ListChannels(ctx, request)
	assert.NoError(t, err)
	assert.NotNil(t, resp)
	assert.NotNil(t, resp.ChannelName)
}

// TestGetChannelDetails_Success verifies successful channel details retrieval.
func TestGetChannelDetails_Success(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseSuccess}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()

	// Save a channel first
	err := dbService.SaveChannel("channel123", []string{"org/ns/mod1/0"})
	require.NoError(t, err)

	channel, err := svc.GetChannelDetails(ctx, "channel123")
	assert.NoError(t, err)
	assert.NotNil(t, channel)
	assert.Equal(t, "channel123", channel.ID)
}

// TestListParticipants_Success verifies successful participant listing.
func TestListParticipants_Success(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseSuccess}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()

	// Save a channel with participants first
	err := dbService.SaveChannel("validParticipantList", []string{"org/ns/mod1/0"})
	require.NoError(t, err)

	channel, err := dbService.GetChannel("validParticipantList")
	require.NoError(t, err)
	channel.Participants = []string{"org/ns/participant1/0", "org/ns/participant2/0"}
	err = dbService.UpdateChannel(channel)
	require.NoError(t, err)

	request := &controllerapi.ListParticipantsRequest{
		ChannelName: "validParticipantList",
	}
	resp, err := svc.ListParticipants(ctx, request)
	assert.NoError(t, err)
	assert.NotNil(t, resp)
	assert.NotNil(t, resp.ParticipantName)
	assert.Len(t, resp.ParticipantName, 2)
}

// TestListParticipants_EmptyChannelName verifies error when channel ID is empty.
func TestListParticipants_EmptyChannelName(t *testing.T) {
	dbService := db.NewInMemoryDBService()
	cmdHandler := &mockNodeCommandHandler{responseType: MockResponseSuccess}
	svc := NewGroupService(dbService, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.ListParticipantsRequest{
		ChannelName: "",
	}
	_, err := svc.ListParticipants(ctx, request)
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "channel ID cannot be empty")
}
