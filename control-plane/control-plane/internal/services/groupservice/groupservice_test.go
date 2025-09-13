// groupservice_test.go - clean, valid tests for groupservice
package groupservice

import (
	"context"
	"errors"
	"testing"

	"github.com/stretchr/testify/assert"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	db "github.com/agntcy/slim/control-plane/control-plane/internal/db"
)

// mockDB implements the data access interface for testing.
type mockDB struct {
	saveChannelErr error
}

func (m *mockDB) SaveChannel(_ string, _ []string) error {
	return m.saveChannelErr
}
func (m *mockDB) DeleteConnection(_ string) error { return nil }

// ListConnectionsByNodeID returns no connections for testing.
// ListConnectionsByNodeID returns no connections for testing.
func (m *mockDB) ListConnectionsByNodeID(_ string) ([]db.Connection, error) {
	return nil, nil
}
func (m *mockDB) SaveConnection(_ db.Connection) (string, error) {
	return "", nil
}
func (m *mockDB) GetConnection(_ string) (db.Connection, error) {
	return db.Connection{}, nil
}

// Stubs for node operations
func (m *mockDB) ListNodes() ([]db.Node, error) { return nil, nil }
func (m *mockDB) GetNode(_ string) (*db.Node, error) {
	return nil, nil
}
func (m *mockDB) SaveNode(_ db.Node) (string, error) {
	return "", nil
}
func (m *mockDB) DeleteNode(_ string) error { return nil }

// Stubs for subscription operations
func (m *mockDB) ListSubscriptionsByNodeID(_ string) ([]db.Subscription, error) {
	return nil, nil
}
func (m *mockDB) SaveSubscription(_ db.Subscription) (string, error) {
	return "", nil
}
func (m *mockDB) GetSubscription(_ string) (db.Subscription, error) {
	return db.Subscription{}, nil
}
func (m *mockDB) DeleteSubscription(_ string) error { return nil }

// Stubs for channel operations
func (m *mockDB) DeleteChannel(_ string) error            { return nil }
func (m *mockDB) GetChannel(_ string) (db.Channel, error) { return db.Channel{}, nil }
func (m *mockDB) UpdateChannel(_ db.Channel) error        { return nil }
func (m *mockDB) ListChannels() ([]db.Channel, error)     { return nil, nil }

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
