package session

import (
	"context"
	"fmt"
	"sync"

	slim "github.com/agntcy/slim/bindings/generated/slim_bindings"
	"github.com/agntcy/slim/bindings/go/examples/common"
	"github.com/agntcy/slim/bindings/go/examples/internal/config"
	slimErrors "github.com/agntcy/slim/bindings/go/examples/internal/errors"
)

// Adapter wraps the SLIM bindings with context support and error handling.
type Adapter struct {
	app     *slim.BindingsAdapter
	connID  uint64
	localID string
	mu      sync.RWMutex
}

// NewAdapter creates a new SLIM adapter and establishes a connection.
func NewAdapter(localID, serverAddr, secret string) (*Adapter, error) {
	app, connID, err := common.CreateAndConnectApp(localID, serverAddr, secret)
	if err != nil {
		return nil, fmt.Errorf("failed to connect to SLIM: %w", err)
	}
	return &Adapter{
		app:     app,
		connID:  connID,
		localID: localID,
	}, nil
}

// LocalID returns the local SLIM identity.
func (a *Adapter) LocalID() string {
	return a.localID
}

// ConnID returns the connection ID.
func (a *Adapter) ConnID() uint64 {
	return a.connID
}

// App returns the underlying SLIM adapter (for operations not wrapped by this adapter).
func (a *Adapter) App() *slim.BindingsAdapter {
	a.mu.RLock()
	defer a.mu.RUnlock()
	return a.app
}

// IsConnected returns true if the adapter is connected.
func (a *Adapter) IsConnected() bool {
	a.mu.RLock()
	defer a.mu.RUnlock()
	return a.app != nil
}

// ListenForSession waits for an incoming session.
// Returns ErrTimeout if no session is received within the timeout.
func (a *Adapter) ListenForSession(ctx context.Context) (*slim.BindingsSessionContext, error) {
	a.mu.RLock()
	app := a.app
	a.mu.RUnlock()

	if app == nil {
		return nil, slimErrors.ErrNotConnected
	}

	timeout := uint32(config.ListenSessionTimeoutMs)

	// Check context before blocking call
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	default:
	}

	session, err := app.ListenForSession(&timeout)
	if err != nil {
		return nil, slimErrors.WrapSLIMError(err)
	}
	return session, nil
}

// CreateSession creates a new session with the given configuration.
func (a *Adapter) CreateSession(ctx context.Context, cfg slim.SessionConfig, destName slim.Name) (*slim.BindingsSessionContext, error) {
	a.mu.RLock()
	app := a.app
	a.mu.RUnlock()

	if app == nil {
		return nil, slimErrors.ErrNotConnected
	}

	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	default:
	}

	session, err := app.CreateSession(cfg, destName)
	if err != nil {
		return nil, slimErrors.WrapSLIMError(err)
	}
	return session, nil
}

// DeleteSession deletes a session.
func (a *Adapter) DeleteSession(ctx context.Context, session *slim.BindingsSessionContext) error {
	a.mu.RLock()
	app := a.app
	a.mu.RUnlock()

	if app == nil {
		return slimErrors.ErrNotConnected
	}

	select {
	case <-ctx.Done():
		return ctx.Err()
	default:
	}

	err := app.DeleteSession(session)
	if err != nil {
		return slimErrors.WrapSLIMError(err)
	}
	return nil
}

// SetRoute sets a route for a destination.
func (a *Adapter) SetRoute(ctx context.Context, destName slim.Name) error {
	a.mu.RLock()
	app := a.app
	connID := a.connID
	a.mu.RUnlock()

	if app == nil {
		return slimErrors.ErrNotConnected
	}

	select {
	case <-ctx.Done():
		return ctx.Err()
	default:
	}

	err := app.SetRoute(destName, connID)
	if err != nil {
		return slimErrors.WrapSLIMError(err)
	}
	return nil
}

// Close closes the adapter and destroys the underlying SLIM connection.
func (a *Adapter) Close() error {
	a.mu.Lock()
	defer a.mu.Unlock()

	if a.app != nil {
		a.app.Destroy()
		a.app = nil
	}
	return nil
}
