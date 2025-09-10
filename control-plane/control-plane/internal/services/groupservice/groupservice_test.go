// groupservice_test.go - clean, valid tests for groupservice
package groupservice

import (
	"context"
	"errors"
	"testing"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	db "github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/stretchr/testify/assert"
)

// mockDB implements the data access interface for testing.
type mockDB struct {
	saveChannelErr error
}

func (m *mockDB) SaveChannel(channelID string, moderators []string) error {
	return m.saveChannelErr
}
func (m *mockDB) DeleteConnection(connectionID string) error { return nil }

// ListConnectionsByNodeID returns no connections for testing.
func (m *mockDB) ListConnectionsByNodeID(nodeID string) ([]db.Connection, error) {
	return nil, nil
}
func (m *mockDB) SaveConnection(connection db.Connection) (string, error) {
	return "", nil
}
func (m *mockDB) GetConnection(connectionID string) (db.Connection, error) {
	return db.Connection{}, nil
}

// Stubs for node operations
func (m *mockDB) ListNodes() ([]db.Node, error) { return nil, nil }
func (m *mockDB) GetNode(id string) (*db.Node, error) {
	return nil, nil
}
func (m *mockDB) SaveNode(node db.Node) (string, error) {
	return "", nil
}
func (m *mockDB) DeleteNode(id string) error { return nil }

// Stubs for subscription operations
func (m *mockDB) ListSubscriptionsByNodeID(nodeID string) ([]db.Subscription, error) {
	return nil, nil
}
func (m *mockDB) SaveSubscription(subscription db.Subscription) (string, error) {
	return "", nil
}
func (m *mockDB) GetSubscription(subscriptionID string) (db.Subscription, error) {
	return db.Subscription{}, nil
}
func (m *mockDB) DeleteSubscription(subscriptionID string) error { return nil }

// Stubs for channel operations
func (m *mockDB) DeleteChannel(channelID string) error            { return nil }
func (m *mockDB) GetChannel(channelID string) (db.Channel, error) { return db.Channel{}, nil }
func (m *mockDB) UpdateChannel(channel db.Channel) error          { return nil }
func (m *mockDB) ListChannels() ([]db.Channel, error)             { return nil, nil }

// TestCreateChannel_Success verifies that creating a channel with moderators succeeds.
func TestCreateChannel_Success(t *testing.T) {
	db := &mockDB{}
	svc := NewGroupService(db)
	ctx := context.Background()
	request := &controllerapi.CreateChannelRequest{
		Moderators: []string{"mod1"},
	}
	resp, err := svc.CreateChannel(ctx, request)
	assert.NoError(t, err)
	assert.NotEmpty(t, resp.ChannelId)
}

// TestCreateChannel_SaveChannelError verifies that an error from SaveChannel is propagated.
func TestCreateChannel_SaveChannelError(t *testing.T) {
	db := &mockDB{saveChannelErr: errors.New("db error")}
	svc := NewGroupService(db)
	ctx := context.Background()
	request := &controllerapi.CreateChannelRequest{
		Moderators: []string{"mod1"},
	}
	_, err := svc.CreateChannel(ctx, request)
	assert.Error(t, err)
}

// TestCreateChannel_NoModerators verifies that missing moderators yields an error.
func TestCreateChannel_NoModerators(t *testing.T) {
	db := &mockDB{}
	svc := NewGroupService(db)
	ctx := context.Background()
	request := &controllerapi.CreateChannelRequest{}
	_, err := svc.CreateChannel(ctx, request)
	assert.Error(t, err)
}
