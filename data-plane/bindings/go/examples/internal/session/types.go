// Package session provides session management types and utilities for SLIM example applications.
package session

import (
	"sync"
	"time"

	slim "github.com/agntcy/slim/bindings/generated/slim_bindings"
)

// Status represents the current state of a session.
type Status string

const (
	// StatusActive indicates the session is active and can send/receive messages.
	StatusActive Status = "active"
	// StatusClosed indicates the session has been closed.
	StatusClosed Status = "closed"
	// StatusError indicates the session encountered an error.
	StatusError Status = "error"
)

// Type represents the type of session.
type Type string

const (
	// TypeP2P indicates a point-to-point session.
	TypeP2P Type = "p2p"
	// TypeGroup indicates a group session.
	TypeGroup Type = "group"
)

// BaseSession contains common session fields shared between sessionmgr and sessionclient.
type BaseSession struct {
	// ID is the unique identifier for this session in the application.
	ID string `json:"id"`

	// SessionID is the SLIM session identifier.
	SessionID uint64 `json:"sessionId"`

	// Type indicates whether this is a p2p or group session.
	Type Type `json:"type"`

	// Destination is the target for p2p sessions or the group identifier.
	Destination string `json:"destination"`

	// Status is the current session status.
	Status Status `json:"status"`

	// CreatedAt is when the session was created.
	CreatedAt time.Time `json:"createdAt"`

	// LastActivity is when the last message was sent or received.
	LastActivity time.Time `json:"lastActivity"`

	// MessageCount is the number of messages in this session.
	MessageCount int `json:"messageCount"`

	// Session is the underlying SLIM session context.
	Session *slim.BindingsSessionContext `json:"-"`

	// sessionMu protects concurrent access to the Session.
	sessionMu sync.Mutex `json:"-"`

	// stopChan is used to signal the message receiver to stop.
	stopChan chan struct{} `json:"-"`

	// cancelFunc is used to cancel the message receiver context.
	cancelFunc func() `json:"-"`
}

// NewBaseSession creates a new BaseSession with initialized fields.
func NewBaseSession(id string, sessionID uint64, sessionType Type, destination string, session *slim.BindingsSessionContext) *BaseSession {
	return &BaseSession{
		ID:           id,
		SessionID:    sessionID,
		Type:         sessionType,
		Destination:  destination,
		Status:       StatusActive,
		CreatedAt:    time.Now(),
		LastActivity: time.Now(),
		Session:      session,
		stopChan:     make(chan struct{}),
	}
}

// Lock acquires the session mutex.
func (s *BaseSession) Lock() {
	s.sessionMu.Lock()
}

// Unlock releases the session mutex.
func (s *BaseSession) Unlock() {
	s.sessionMu.Unlock()
}

// Stop signals the message receiver to stop and cancels the context.
func (s *BaseSession) Stop() {
	if s.cancelFunc != nil {
		s.cancelFunc()
	}
	select {
	case <-s.stopChan:
		// Already closed
	default:
		close(s.stopChan)
	}
}

// IsStopped returns true if the session has been stopped.
func (s *BaseSession) IsStopped() bool {
	select {
	case <-s.stopChan:
		return true
	default:
		return false
	}
}

// SetCancelFunc sets the cancel function for the message receiver context.
func (s *BaseSession) SetCancelFunc(cancel func()) {
	s.cancelFunc = cancel
}

// UpdateActivity updates the last activity timestamp and increments the message count.
func (s *BaseSession) UpdateActivity() {
	s.LastActivity = time.Now()
	s.MessageCount++
}

// SetStatus updates the session status.
func (s *BaseSession) SetStatus(status Status) {
	s.Status = status
}
