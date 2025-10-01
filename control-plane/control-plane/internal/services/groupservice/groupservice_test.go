// groupservice_test.go - clean, valid tests for groupservice
package groupservice

import (
	"context"
	"errors"
	"reflect"
	"testing"

	"github.com/stretchr/testify/assert"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	db "github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
)

// mockDB implements the data access interface for testing.
type mockDB struct {
	saveChannelErr error
}

// AddRoute implements db.DataAccess.
func (m *mockDB) AddRoute(_ db.Route) string {
	panic("unimplemented")
}

// DeleteRoute implements db.DataAccess.
func (m *mockDB) DeleteRoute(_ string) error {
	panic("unimplemented")
}

// GetRouteByID implements db.DataAccess.
func (m *mockDB) GetRouteByID(_ string) *db.Route {
	panic("unimplemented")
}

// GetRoutesForNodeID implements db.DataAccess.
func (m *mockDB) GetRoutesForNodeID(_ string) []db.Route {
	panic("unimplemented")
}

// MarkRouteAsDeleted implements db.DataAccess.
func (m *mockDB) MarkRouteAsDeleted(_ string) error {
	panic("unimplemented")
}

func (m *mockDB) SaveChannel(_ string, _ []string) error { return m.saveChannelErr }
func (m *mockDB) DeleteConnection(_ string) error        { return nil }

// Stubs for node operations
func (m *mockDB) ListNodes() []db.Node               { return nil }
func (m *mockDB) GetNode(_ string) (*db.Node, error) { return nil, nil }
func (m *mockDB) SaveNode(_ db.Node) (string, error) { return "", nil }
func (m *mockDB) DeleteNode(_ string) error          { return nil }

// Stubs for channel operations
func (m *mockDB) DeleteChannel(_ string) error        { return nil }
func (m *mockDB) UpdateChannel(_ db.Channel) error    { return nil }
func (m *mockDB) ListChannels() ([]db.Channel, error) { return nil, nil }

func (m *mockDB) GetChannel(channelName string) (db.Channel, error) {
	if channelName == "validParticipantList" {
		return db.Channel{
			ID:           "validParticipantList",
			Moderators:   []string{"org/ns/mod1/0"},
			Participants: []string{"org/ns/participant1/0", "org/ns/participant2/0"},
		}, nil
	}

	if channelName == "deleteParticipantChannel" {
		return db.Channel{
			ID:           "deleteParticipantChannel",
			Moderators:   []string{"org/ns/mod1/0"},
			Participants: []string{"org/ns/participantToDelete/0", "org/ns/participant2/0"},
		}, nil
	}

	return db.Channel{}, nil
}

type mockNodeCommandHandler struct{}

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
	return nil, nil
}

func (m *mockNodeCommandHandler) ResponseReceived(
	_ context.Context, _ string, _ *controllerapi.ControlMessage) {
}

// TestCreateChannel_Success verifies that creating a channel with moderators succeeds.
func TestCreateChannel_Success(t *testing.T) {
	db := &mockDB{}
	cmdHandler := &mockNodeCommandHandler{}
	svc := NewGroupService(db, cmdHandler)
	ctx := context.Background()
	nodeEntry := &controlplaneApi.NodeEntry{Id: "node123"}

	// create a valid request
	request := &controlplaneApi.CreateChannelRequest{
		Moderators: []string{"org/ns/mod/0"},
	}
	resp, err := svc.CreateChannel(ctx, request, nodeEntry)
	assert.NoError(t, err)
	assert.NotEmpty(t, resp.ChannelName)
	assert.Contains(t, resp.ChannelName, "org/ns/")
}

func TestCreateChannel_InvalidModeratorName(t *testing.T) {
	db := &mockDB{}
	cmdHandler := &mockNodeCommandHandler{}
	svc := NewGroupService(db, cmdHandler)
	ctx := context.Background()
	nodeEntry := &controlplaneApi.NodeEntry{Id: "node123"}

	// create a valid request
	request := &controlplaneApi.CreateChannelRequest{
		Moderators: []string{"asd"},
	}
	resp, err := svc.CreateChannel(ctx, request, nodeEntry)
	assert.Error(t, err)
	assert.Nil(t, resp)
	assert.Contains(t, err.Error(), "invalid moderator name")
}

// TestCreateChannel_SaveChannelError verifies that an error from SaveChannel is propagated.
func TestCreateChannel_SaveChannelError(t *testing.T) {
	db := &mockDB{saveChannelErr: errors.New("db error")}
	cmdHandler := &mockNodeCommandHandler{}
	svc := NewGroupService(db, cmdHandler)
	ctx := context.Background()
	request := &controlplaneApi.CreateChannelRequest{
		Moderators: []string{"org/ns/mod1/0"},
	}
	nodeEntry := &controlplaneApi.NodeEntry{Id: "node123"}
	_, err := svc.CreateChannel(ctx, request, nodeEntry)
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "failed to save channel")
}

// TestCreateChannel_NoModerators verifies that missing moderators yields an error.
func TestCreateChannel_NoModerators(t *testing.T) {
	db := &mockDB{}
	cmdHandler := &mockNodeCommandHandler{}
	svc := NewGroupService(db, cmdHandler)
	ctx := context.Background()
	request := &controlplaneApi.CreateChannelRequest{}
	_, err := svc.CreateChannel(ctx, request, nil)
	assert.Error(t, err)
}

// TestCreateChannel_NilNodeEntry verifies that nil nodeEntry causes panic.
func TestCreateChannel_NilNodeEntry(t *testing.T) {
	db := &mockDB{}
	cmdHandler := &mockNodeCommandHandler{}
	svc := NewGroupService(db, cmdHandler)
	ctx := context.Background()
	request := &controlplaneApi.CreateChannelRequest{
		Moderators: []string{"org/ns/mod1/0"},
	}
	assert.Panics(t, func() {
		_, _ = svc.CreateChannel(ctx, request, nil)
	})
}

// TestDeleteChannel_Success verifies successful channel deletion.
func TestDeleteChannel_Success(t *testing.T) {
	db := &mockDB{}
	cmdHandler := &mockNodeCommandHandler{}
	svc := NewGroupService(db, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.DeleteChannelRequest{
		ChannelName: "org/ns/channel123",
	}
	nodeEntry := &controlplaneApi.NodeEntry{Id: "node123"}
	resp, err := svc.DeleteChannel(ctx, request, nodeEntry)
	assert.NoError(t, err)
	assert.True(t, resp.Success)
}

// TestDeleteChannel_EmptyChannelName verifies error when channel ID is empty.
func TestDeleteChannel_EmptyChannelName(t *testing.T) {
	db := &mockDB{}
	cmdHandler := &mockNodeCommandHandler{}
	svc := NewGroupService(db, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.DeleteChannelRequest{
		ChannelName: "",
	}
	nodeEntry := &controlplaneApi.NodeEntry{Id: "node123"}
	_, err := svc.DeleteChannel(ctx, request, nodeEntry)
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "name cannot be empty")
}

// TestAddParticipant_Success verifies successful participant addition.
func TestAddParticipant_Success(t *testing.T) {
	db := &mockDB{}
	cmdHandler := &mockNodeCommandHandler{}
	svc := NewGroupService(db, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.AddParticipantRequest{
		ChannelName:   "org/ns/channel123",
		ParticipantId: "org/ns/participant123/0",
	}
	nodeEntry := &controlplaneApi.NodeEntry{Id: "node123"}
	resp, err := svc.AddParticipant(ctx, request, nodeEntry)
	assert.NoError(t, err)
	assert.True(t, resp.Success)
}

// TestAddParticipant_EmptyChannelName verifies error when channel ID is empty.
func TestAddParticipant_EmptyChannelName(t *testing.T) {
	db := &mockDB{}
	cmdHandler := &mockNodeCommandHandler{}
	svc := NewGroupService(db, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.AddParticipantRequest{
		ChannelName:   "",
		ParticipantId: "participant123",
	}
	nodeEntry := &controlplaneApi.NodeEntry{Id: "node123"}
	_, err := svc.AddParticipant(ctx, request, nodeEntry)
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "name cannot be empty")
}

// TestAddParticipant_EmptyParticipantID verifies error when participant ID is empty.
func TestAddParticipant_EmptyParticipantID(t *testing.T) {
	db := &mockDB{}
	cmdHandler := &mockNodeCommandHandler{}
	svc := NewGroupService(db, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.AddParticipantRequest{
		ChannelName:   "org/name/channel123",
		ParticipantId: "",
	}
	nodeEntry := &controlplaneApi.NodeEntry{Id: "node123"}
	_, err := svc.AddParticipant(ctx, request, nodeEntry)
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "name cannot be empty")
}

// TestDeleteParticipant_Success verifies successful participant deletion.
func TestDeleteParticipant_Success(t *testing.T) {
	db := &mockDB{}
	cmdHandler := &mockNodeCommandHandler{}
	svc := NewGroupService(db, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.DeleteParticipantRequest{
		ChannelName:   "deleteParticipantChannel",
		ParticipantId: "org/ns/participantToDelete/0",
	}
	nodeEntry := &controlplaneApi.NodeEntry{Id: "node123"}
	resp, err := svc.DeleteParticipant(ctx, request, nodeEntry)
	assert.NoError(t, err)
	assert.True(t, resp.Success)
}

// TestDeleteParticipant_EmptyChannelName verifies error when channel ID is empty.
func TestDeleteParticipant_EmptyChannelName(t *testing.T) {
	db := &mockDB{}
	cmdHandler := &mockNodeCommandHandler{}
	svc := NewGroupService(db, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.DeleteParticipantRequest{
		ChannelName:   "",
		ParticipantId: "participant123",
	}
	nodeEntry := &controlplaneApi.NodeEntry{Id: "node123"}
	_, err := svc.DeleteParticipant(ctx, request, nodeEntry)
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "channel ID cannot be empty")
}

// TestDeleteParticipant_EmptyParticipantID verifies error when participant ID is empty.
func TestDeleteParticipant_EmptyParticipantID(t *testing.T) {
	db := &mockDB{}
	cmdHandler := &mockNodeCommandHandler{}
	svc := NewGroupService(db, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.DeleteParticipantRequest{
		ChannelName:   "channel123",
		ParticipantId: "",
	}
	nodeEntry := &controlplaneApi.NodeEntry{Id: "node123"}
	_, err := svc.DeleteParticipant(ctx, request, nodeEntry)
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "participant ID cannot be empty")
}

// TestListChannels_Success verifies successful channel listing.
func TestListChannels_Success(t *testing.T) {
	db := &mockDB{}
	cmdHandler := &mockNodeCommandHandler{}
	svc := NewGroupService(db, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.ListChannelsRequest{}
	resp, err := svc.ListChannels(ctx, request)
	assert.NoError(t, err)
	assert.NotNil(t, resp)
	assert.NotNil(t, resp.ChannelName)
}

// TestGetChannelDetails_Success verifies successful channel details retrieval.
func TestGetChannelDetails_Success(t *testing.T) {
	db := &mockDB{}
	cmdHandler := &mockNodeCommandHandler{}
	svc := NewGroupService(db, cmdHandler)
	ctx := context.Background()
	channel, err := svc.GetChannelDetails(ctx, "channel123")
	assert.NoError(t, err)
	assert.NotNil(t, channel)
}

// TestListParticipants_Success verifies successful participant listing.
func TestListParticipants_Success(t *testing.T) {
	db := &mockDB{}
	cmdHandler := &mockNodeCommandHandler{}
	svc := NewGroupService(db, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.ListParticipantsRequest{
		ChannelName: "validParticipantList",
	}
	resp, err := svc.ListParticipants(ctx, request)
	assert.NoError(t, err)
	assert.NotNil(t, resp)
	assert.NotNil(t, resp.ParticipantId)
}

// TestListParticipants_EmptyChannelName verifies error when channel ID is empty.
func TestListParticipants_EmptyChannelName(t *testing.T) {
	db := &mockDB{}
	cmdHandler := &mockNodeCommandHandler{}
	svc := NewGroupService(db, cmdHandler)
	ctx := context.Background()
	request := &controllerapi.ListParticipantsRequest{
		ChannelName: "",
	}
	_, err := svc.ListParticipants(ctx, request)
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "channel ID cannot be empty")
}
