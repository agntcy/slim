// Package manager provides the SessionManager business logic for sessionmgr.
package manager

import (
	"context"
	"fmt"
	"log"
	"strings"
	"sync"
	"time"

	slim "github.com/agntcy/slim/bindings/generated/slim_bindings"
	"github.com/agntcy/slim/bindings/go/examples/common"
	"github.com/agntcy/slim/bindings/go/examples/internal/config"
	slimErrors "github.com/agntcy/slim/bindings/go/examples/internal/errors"
	"github.com/agntcy/slim/bindings/go/examples/internal/httputil"
	"github.com/agntcy/slim/bindings/go/examples/internal/message"
)

// SessionInfo represents an active SLIM session with its metadata.
type SessionInfo struct {
	ID           string                       `json:"id"`
	SessionID    uint32                       `json:"sessionId"`
	Type         string                       `json:"type"` // "p2p" or "group"
	Destination  string                       `json:"destination"`
	IsInitiator  bool                         `json:"isInitiator"`
	Participants []string                     `json:"participants"`
	MessageCount int                          `json:"messageCount"`
	CreatedAt    time.Time                    `json:"createdAt"`
	LastActivity time.Time                    `json:"lastActivity"`
	Status       string                       `json:"status"` // "active", "closed", "error"
	EnableMLS    bool                         `json:"enableMls"`
	Session      *slim.BindingsSessionContext `json:"-"`
	sessionMu    sync.Mutex                   `json:"-"`
	stopChan     chan struct{}                `json:"-"`
}

// KnownClient represents a discovered/known client for invite suggestions.
type KnownClient struct {
	Name     string    `json:"name"`
	LastSeen time.Time `json:"lastSeen"`
	Source   string    `json:"source"` // "message", "invite", "manual"
}

// Manager holds all session management state.
type Manager struct {
	app      *slim.BindingsAdapter
	connID   uint64
	localID  string
	sessions map[string]*SessionInfo
	messages *message.InMemoryStore
	clients  map[string]*KnownClient
	mu       sync.RWMutex

	// SSE broadcasters
	globalSSE  httputil.Broadcaster
	sessionSSE httputil.SessionBroadcaster
}

// New creates a new Manager.
func New(app *slim.BindingsAdapter, connID uint64, localID string, globalSSE httputil.Broadcaster, sessionSSE httputil.SessionBroadcaster) *Manager {
	return &Manager{
		app:        app,
		connID:     connID,
		localID:    localID,
		sessions:   make(map[string]*SessionInfo),
		messages:   message.NewInMemoryStore(),
		clients:    make(map[string]*KnownClient),
		globalSSE:  globalSSE,
		sessionSSE: sessionSSE,
	}
}

// LocalID returns the local SLIM identity.
func (m *Manager) LocalID() string {
	return m.localID
}

// IsConnected returns true if the manager has a SLIM connection.
func (m *Manager) IsConnected() bool {
	return m.app != nil
}

// GetSessions returns a copy of all sessions.
func (m *Manager) GetSessions() []*SessionInfo {
	m.mu.RLock()
	defer m.mu.RUnlock()

	sessions := make([]*SessionInfo, 0, len(m.sessions))
	for _, s := range m.sessions {
		sessions = append(sessions, s)
	}
	return sessions
}

// GetSession returns a session by ID.
func (m *Manager) GetSession(sessionID string) (*SessionInfo, bool) {
	m.mu.RLock()
	defer m.mu.RUnlock()
	s, ok := m.sessions[sessionID]
	return s, ok
}

// GetMessages returns messages for a session.
func (m *Manager) GetMessages(sessionID string) []message.Message {
	return m.messages.Get(sessionID)
}

// GetClients returns a copy of all known clients.
func (m *Manager) GetClients() []*KnownClient {
	m.mu.RLock()
	defer m.mu.RUnlock()

	clients := make([]*KnownClient, 0, len(m.clients))
	for _, c := range m.clients {
		clients = append(clients, c)
	}
	return clients
}

// CreateSession creates a new SLIM session.
func (m *Manager) CreateSession(ctx context.Context, sessionType, destination string, enableMLS bool) (*SessionInfo, error) {
	destName, err := common.SplitID(destination)
	if err != nil {
		return nil, slimErrors.ErrInvalidDestination
	}

	// Determine SLIM session type
	slimType := slim.SessionTypePointToPoint
	if sessionType == "group" {
		slimType = slim.SessionTypeGroup
	}

	var session *slim.BindingsSessionContext
	var sessionID uint32

	if m.app != nil {
		// Set route for destination
		if err := m.app.SetRoute(destName, m.connID); err != nil {
			return nil, fmt.Errorf("failed to set route: %w", err)
		}

		slimConfig := slim.SessionConfig{
			SessionType: slimType,
			EnableMls:   enableMLS,
			MaxRetries:  &[]uint32{5}[0],
			IntervalMs:  &[]uint64{5000}[0],
			Initiator:   true,
			Metadata:    make(map[string]string),
		}

		session, err = m.app.CreateSession(slimConfig, destName)
		if err != nil {
			return nil, fmt.Errorf("failed to create session: %w", err)
		}

		// Give session a moment to establish
		time.Sleep(config.SessionEstablishDelay)

		sessionID, _ = session.SessionId()
	} else {
		// Demo mode
		sessionID = uint32(time.Now().UnixNano() % 1000000)
	}

	info := &SessionInfo{
		ID:           fmt.Sprintf("%s-%d", sessionType, sessionID),
		SessionID:    sessionID,
		Type:         sessionType,
		Destination:  destination,
		IsInitiator:  true,
		Participants: []string{},
		CreatedAt:    time.Now(),
		LastActivity: time.Now(),
		Status:       "active",
		EnableMLS:    enableMLS,
		Session:      session,
		stopChan:     make(chan struct{}),
	}

	m.mu.Lock()
	m.sessions[info.ID] = info
	m.mu.Unlock()

	// Start message receiver
	if session != nil {
		go m.receiveMessages(info)
	}

	// Broadcast SSE event
	m.globalSSE.Broadcast(httputil.SSEEvent{
		Type: "session_created",
		Data: map[string]interface{}{"id": info.ID, "type": info.Type, "destination": info.Destination},
	})

	return info, nil
}

// DeleteSession deletes a session.
func (m *Manager) DeleteSession(ctx context.Context, sessionID string) error {
	m.mu.Lock()
	info, exists := m.sessions[sessionID]
	if !exists {
		m.mu.Unlock()
		return slimErrors.ErrSessionNotFound
	}
	delete(m.sessions, sessionID)
	m.mu.Unlock()

	m.messages.Delete(sessionID)

	// Stop message receiver
	if info.stopChan != nil {
		close(info.stopChan)
	}

	// Delete SLIM session
	if m.app != nil && info.Session != nil {
		if err := m.app.DeleteSession(info.Session); err != nil {
			log.Printf("Warning: failed to delete SLIM session: %v", err)
		}
	}

	// Clean up SSE subscriptions
	m.sessionSSE.DeleteSession(sessionID)

	m.globalSSE.Broadcast(httputil.SSEEvent{
		Type: "session_deleted",
		Data: map[string]string{"id": sessionID},
	})

	return nil
}

// InviteParticipant invites a participant to a group session.
func (m *Manager) InviteParticipant(ctx context.Context, sessionID, participant string) error {
	m.mu.RLock()
	info, exists := m.sessions[sessionID]
	m.mu.RUnlock()

	if !exists {
		return slimErrors.ErrSessionNotFound
	}
	if info.Type != "group" {
		return slimErrors.ErrNotGroupSession
	}
	if !info.IsInitiator {
		return slimErrors.ErrNotInitiator
	}

	participantName, err := common.SplitID(participant)
	if err != nil {
		return slimErrors.ErrInvalidDestination
	}

	if m.app != nil && info.Session != nil {
		// Set route for participant
		if err := m.app.SetRoute(participantName, m.connID); err != nil {
			return fmt.Errorf("failed to set route: %w", err)
		}

		// Send invite
		info.sessionMu.Lock()
		err := info.Session.Invite(participantName)
		info.sessionMu.Unlock()
		if err != nil {
			return fmt.Errorf("failed to invite: %w", err)
		}
	}

	// Update participants list
	m.mu.Lock()
	info.Participants = append(info.Participants, participant)
	m.mu.Unlock()

	// Track as known client
	m.addKnownClient(participant, "invite")

	m.globalSSE.Broadcast(httputil.SSEEvent{
		Type: "participant_joined",
		Data: map[string]string{"sessionId": sessionID, "participant": participant},
	})

	return nil
}

// RemoveParticipant removes a participant from a group session.
func (m *Manager) RemoveParticipant(ctx context.Context, sessionID, participant string) error {
	m.mu.RLock()
	info, exists := m.sessions[sessionID]
	m.mu.RUnlock()

	if !exists {
		return slimErrors.ErrSessionNotFound
	}
	if info.Type != "group" {
		return slimErrors.ErrNotGroupSession
	}
	if !info.IsInitiator {
		return slimErrors.ErrNotInitiator
	}

	participantName, err := common.SplitID(participant)
	if err != nil {
		return slimErrors.ErrInvalidDestination
	}

	if m.app != nil && info.Session != nil {
		info.sessionMu.Lock()
		err := info.Session.Remove(participantName)
		info.sessionMu.Unlock()
		if err != nil {
			return fmt.Errorf("failed to remove: %w", err)
		}
	}

	// Update participants list
	m.mu.Lock()
	newParticipants := make([]string, 0, len(info.Participants))
	for _, p := range info.Participants {
		if p != participant {
			newParticipants = append(newParticipants, p)
		}
	}
	info.Participants = newParticipants
	m.mu.Unlock()

	m.globalSSE.Broadcast(httputil.SSEEvent{
		Type: "participant_removed",
		Data: map[string]string{"sessionId": sessionID, "participant": participant},
	})

	return nil
}

// SendMessage sends a message in a session.
func (m *Manager) SendMessage(ctx context.Context, sessionID, text string) error {
	m.mu.RLock()
	info, exists := m.sessions[sessionID]
	m.mu.RUnlock()

	if !exists {
		return slimErrors.ErrSessionNotFound
	}

	if m.app != nil && info.Session != nil {
		info.sessionMu.Lock()
		err := info.Session.Publish([]byte(text), nil, nil)
		info.sessionMu.Unlock()
		if err != nil {
			return fmt.Errorf("failed to publish: %w", err)
		}
	}

	msg := message.Message{
		ID:        fmt.Sprintf("msg-%d", time.Now().UnixNano()),
		SessionID: sessionID,
		Direction: message.DirectionSent,
		Sender:    "you",
		Text:      text,
		Timestamp: time.Now(),
	}

	m.messages.Add(sessionID, msg)

	m.mu.Lock()
	info.MessageCount++
	info.LastActivity = time.Now()
	m.mu.Unlock()

	// Broadcast via SSE
	m.sessionSSE.Broadcast(sessionID, httputil.SSEEvent{
		Type: "message",
		Data: formatMessageHTML(msg),
	})

	return nil
}

// ListenForIncomingSessions listens for incoming sessions.
func (m *Manager) ListenForIncomingSessions(ctx context.Context) {
	if m.app == nil {
		return
	}

	for {
		select {
		case <-ctx.Done():
			return
		default:
		}

		timeout := uint32(config.ListenSessionTimeoutMs)
		session, err := m.app.ListenForSession(&timeout)
		if err != nil {
			wrappedErr := slimErrors.WrapSLIMError(err)
			if slimErrors.IsTimeout(wrappedErr) {
				continue
			}
			log.Printf("Listen error: %v", err)
			continue
		}

		// Accept the incoming session
		dest, _ := session.Destination()
		sessionID, _ := session.SessionId()
		sessionType, _ := session.SessionType()
		isInit, _ := session.IsInitiator()

		typeStr := "p2p"
		if sessionType == slim.SessionTypeGroup {
			typeStr = "group"
		}

		info := &SessionInfo{
			ID:           fmt.Sprintf("%s-%d", typeStr, sessionID),
			SessionID:    sessionID,
			Type:         typeStr,
			Destination:  strings.Join(dest.Components, "/"),
			IsInitiator:  isInit,
			Participants: []string{},
			CreatedAt:    time.Now(),
			LastActivity: time.Now(),
			Status:       "active",
			Session:      session,
			stopChan:     make(chan struct{}),
		}

		m.mu.Lock()
		m.sessions[info.ID] = info
		m.mu.Unlock()

		log.Printf("Accepted incoming session: %s (%s)", info.ID, info.Destination)

		// Start message receiver
		go m.receiveMessages(info)

		m.globalSSE.Broadcast(httputil.SSEEvent{
			Type: "session_created",
			Data: map[string]interface{}{"id": info.ID, "type": info.Type, "destination": info.Destination, "incoming": true},
		})
	}
}

func (m *Manager) receiveMessages(info *SessionInfo) {
	for {
		select {
		case <-info.stopChan:
			return
		default:
		}

		timeout := uint32(config.GetMessageTimeoutMs)
		info.sessionMu.Lock()
		msg, err := info.Session.GetMessage(&timeout)
		info.sessionMu.Unlock()

		if err != nil {
			wrappedErr := slimErrors.WrapSLIMError(err)
			if slimErrors.IsTimeout(wrappedErr) {
				continue
			}
			if slimErrors.IsParticipantDisconnected(wrappedErr) {
				log.Printf("Session %s: %v", info.ID, err)
				continue
			}
			// Session closed or fatal error
			log.Printf("Session %s GetMessage error: %v", info.ID, err)
			m.mu.Lock()
			info.Status = "closed"
			m.mu.Unlock()

			m.globalSSE.Broadcast(httputil.SSEEvent{
				Type: "session_updated",
				Data: map[string]string{"id": info.ID, "status": "closed"},
			})
			return
		}

		// Create message record
		sender := strings.Join(msg.Context.SourceName.Components, "/")
		message := message.Message{
			ID:        fmt.Sprintf("msg-%d", time.Now().UnixNano()),
			SessionID: info.ID,
			Direction: message.DirectionReceived,
			Sender:    sender,
			Text:      string(msg.Payload),
			Metadata:  msg.Context.Metadata,
			Timestamp: time.Now(),
		}

		m.messages.Add(info.ID, message)

		m.mu.Lock()
		info.MessageCount++
		info.LastActivity = time.Now()
		m.mu.Unlock()

		// Track sender as known client
		m.addKnownClient(sender, "message")

		// Broadcast message via SSE
		m.sessionSSE.Broadcast(info.ID, httputil.SSEEvent{
			Type: "message",
			Data: formatMessageHTML(message),
		})
	}
}

func (m *Manager) addKnownClient(name, source string) {
	m.mu.Lock()
	defer m.mu.Unlock()

	m.clients[name] = &KnownClient{
		Name:     name,
		LastSeen: time.Now(),
		Source:   source,
	}
}

// formatMessageHTML formats a message as HTML for SSE.
func formatMessageHTML(msg message.Message) string {
	timestamp := msg.Timestamp.Format("15:04:05")
	class := "message"
	if msg.Direction == message.DirectionSent {
		class = "message sent"
	}
	// HTML escape is done by template package, but for SSE we do it manually
	return fmt.Sprintf(`<div class="%s"><span class="time">[%s]</span> <span class="sender">%s:</span> %s</div>`,
		class, timestamp, msg.Sender, msg.Text)
}
