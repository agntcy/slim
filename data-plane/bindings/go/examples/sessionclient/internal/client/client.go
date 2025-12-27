// Package client provides the SessionClient business logic for sessionclient.
package client

import (
	"context"
	"fmt"
	"log"
	"strings"
	"sync"
	"time"

	slim "github.com/agntcy/slim/bindings/generated/slim_bindings"
	"github.com/agntcy/slim/bindings/go/examples/internal/config"
	slimErrors "github.com/agntcy/slim/bindings/go/examples/internal/errors"
	"github.com/agntcy/slim/bindings/go/examples/internal/httputil"
	"github.com/agntcy/slim/bindings/go/examples/internal/message"
)

// PendingInvite represents an incoming session invitation.
type PendingInvite struct {
	ID          string                       `json:"id"`
	SessionID   uint32                       `json:"sessionId"`
	Type        string                       `json:"type"` // "p2p" or "group"
	From        string                       `json:"from"` // Inviter's identity
	Destination string                       `json:"destination"`
	ReceivedAt  time.Time                    `json:"receivedAt"`
	ExpiresAt   time.Time                    `json:"expiresAt"`
	Session     *slim.BindingsSessionContext `json:"-"`
}

// TimeRemaining returns seconds until expiry.
func (p *PendingInvite) TimeRemaining() int {
	remaining := int(time.Until(p.ExpiresAt).Seconds())
	if remaining < 0 {
		return 0
	}
	return remaining
}

// JoinedSession represents an accepted session.
type JoinedSession struct {
	ID           string                       `json:"id"`
	SessionID    uint32                       `json:"sessionId"`
	Type         string                       `json:"type"` // "p2p" or "group"
	Destination  string                       `json:"destination"`
	JoinedAt     time.Time                    `json:"joinedAt"`
	MessageCount int                          `json:"messageCount"`
	Status       string                       `json:"status"` // "active", "closed"
	Session      *slim.BindingsSessionContext `json:"-"`
	sessionMu    sync.Mutex                   `json:"-"`
	stopChan     chan struct{}                `json:"-"`
}

// Client holds all session client state.
type Client struct {
	app            *slim.BindingsAdapter
	connID         uint64
	localID        string
	inviteTimeout  int
	pendingInvites map[string]*PendingInvite
	joinedSessions map[string]*JoinedSession
	messages       *message.InMemoryStore
	mu             sync.RWMutex

	// SSE broadcasters
	globalSSE  httputil.Broadcaster
	sessionSSE httputil.SessionBroadcaster

	// Stop channel for cleanup goroutine
	stopCleanup chan struct{}
}

// New creates a new Client.
func New(app *slim.BindingsAdapter, connID uint64, localID string, inviteTimeout int, globalSSE httputil.Broadcaster, sessionSSE httputil.SessionBroadcaster) *Client {
	return &Client{
		app:            app,
		connID:         connID,
		localID:        localID,
		inviteTimeout:  inviteTimeout,
		pendingInvites: make(map[string]*PendingInvite),
		joinedSessions: make(map[string]*JoinedSession),
		messages:       message.NewInMemoryStore(),
		globalSSE:      globalSSE,
		sessionSSE:     sessionSSE,
		stopCleanup:    make(chan struct{}),
	}
}

// LocalID returns the local SLIM identity.
func (c *Client) LocalID() string {
	return c.localID
}

// IsConnected returns true if the client has a SLIM connection.
func (c *Client) IsConnected() bool {
	return c.app != nil
}

// GetPendingInvites returns a copy of all pending invitations.
func (c *Client) GetPendingInvites() []*PendingInvite {
	c.mu.RLock()
	defer c.mu.RUnlock()

	invites := make([]*PendingInvite, 0, len(c.pendingInvites))
	for _, inv := range c.pendingInvites {
		invites = append(invites, inv)
	}
	return invites
}

// GetJoinedSessions returns a copy of all joined sessions.
func (c *Client) GetJoinedSessions() []*JoinedSession {
	c.mu.RLock()
	defer c.mu.RUnlock()

	sessions := make([]*JoinedSession, 0, len(c.joinedSessions))
	for _, s := range c.joinedSessions {
		sessions = append(sessions, s)
	}
	return sessions
}

// GetSession returns a session by ID.
func (c *Client) GetSession(sessionID string) (*JoinedSession, bool) {
	c.mu.RLock()
	defer c.mu.RUnlock()
	s, ok := c.joinedSessions[sessionID]
	return s, ok
}

// GetMessages returns messages for a session.
func (c *Client) GetMessages(sessionID string) []message.Message {
	return c.messages.Get(sessionID)
}

// AcceptInvitation accepts a pending invitation.
func (c *Client) AcceptInvitation(ctx context.Context, inviteID string) error {
	c.mu.Lock()
	invite, exists := c.pendingInvites[inviteID]
	if !exists {
		c.mu.Unlock()
		return slimErrors.ErrInvitationNotFound
	}
	delete(c.pendingInvites, inviteID)
	c.mu.Unlock()

	// Check if expired
	if time.Now().After(invite.ExpiresAt) {
		if c.app != nil && invite.Session != nil {
			c.app.DeleteSession(invite.Session)
		}
		return slimErrors.ErrInvitationExpired
	}

	// Create joined session
	sessionID := fmt.Sprintf("%s-%d", invite.Type, invite.SessionID)
	joined := &JoinedSession{
		ID:           sessionID,
		SessionID:    invite.SessionID,
		Type:         invite.Type,
		Destination:  invite.Destination,
		JoinedAt:     time.Now(),
		MessageCount: 0,
		Status:       "active",
		Session:      invite.Session,
		stopChan:     make(chan struct{}),
	}

	c.mu.Lock()
	c.joinedSessions[sessionID] = joined
	c.mu.Unlock()

	// Start message receiver after a brief delay
	if invite.Session != nil {
		time.Sleep(config.SessionEstablishDelay)
		go c.receiveMessages(joined)
	}

	c.globalSSE.Broadcast(httputil.SSEEvent{
		Type: "session_joined",
		Data: map[string]interface{}{
			"id":          sessionID,
			"type":        joined.Type,
			"destination": joined.Destination,
		},
	})

	return nil
}

// DeclineInvitation declines a pending invitation.
func (c *Client) DeclineInvitation(ctx context.Context, inviteID string) error {
	c.mu.Lock()
	invite, exists := c.pendingInvites[inviteID]
	if !exists {
		c.mu.Unlock()
		return slimErrors.ErrInvitationNotFound
	}
	delete(c.pendingInvites, inviteID)
	c.mu.Unlock()

	// Delete the session to clean up
	if c.app != nil && invite.Session != nil {
		c.app.DeleteSession(invite.Session)
	}

	c.globalSSE.Broadcast(httputil.SSEEvent{
		Type: "invitation_expired",
		Data: map[string]string{"id": inviteID},
	})

	return nil
}

// LeaveSession leaves a joined session.
func (c *Client) LeaveSession(ctx context.Context, sessionID string) error {
	c.mu.Lock()
	session, exists := c.joinedSessions[sessionID]
	if !exists {
		c.mu.Unlock()
		return slimErrors.ErrSessionNotFound
	}
	delete(c.joinedSessions, sessionID)
	c.mu.Unlock()

	c.messages.Delete(sessionID)

	// Stop message receiver
	if session.stopChan != nil {
		close(session.stopChan)
	}

	// Delete SLIM session
	if c.app != nil && session.Session != nil {
		c.app.DeleteSession(session.Session)
	}

	// Clean up SSE subscriptions
	c.sessionSSE.DeleteSession(sessionID)

	c.globalSSE.Broadcast(httputil.SSEEvent{
		Type: "session_left",
		Data: map[string]string{"id": sessionID},
	})

	return nil
}

// SendMessage sends a message in a session.
func (c *Client) SendMessage(ctx context.Context, sessionID, text string) error {
	c.mu.RLock()
	session, exists := c.joinedSessions[sessionID]
	c.mu.RUnlock()

	if !exists {
		return slimErrors.ErrSessionNotFound
	}

	if c.app != nil && session.Session != nil {
		session.sessionMu.Lock()
		err := session.Session.Publish([]byte(text), nil, nil)
		session.sessionMu.Unlock()
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

	c.messages.Add(sessionID, msg)

	c.mu.Lock()
	session.MessageCount++
	c.mu.Unlock()

	c.sessionSSE.Broadcast(sessionID, httputil.SSEEvent{
		Type: "message",
		Data: formatMessageHTML(msg),
	})

	return nil
}

// ListenForInvitations listens for incoming session invitations.
func (c *Client) ListenForInvitations(ctx context.Context) {
	if c.app == nil {
		return
	}

	for {
		select {
		case <-ctx.Done():
			return
		default:
		}

		timeout := uint32(config.ListenSessionTimeoutMs)
		session, err := c.app.ListenForSession(&timeout)
		if err != nil {
			wrappedErr := slimErrors.WrapSLIMError(err)
			if slimErrors.IsTimeout(wrappedErr) {
				continue
			}
			log.Printf("Listen error: %v", err)
			continue
		}

		// Get session info
		dest, _ := session.Destination()
		sessionID, _ := session.SessionId()
		sessionType, _ := session.SessionType()
		source, _ := session.Source()

		typeStr := "p2p"
		if sessionType == slim.SessionTypeGroup {
			typeStr = "group"
		}

		fromStr := ""
		if source.Components != nil && len(source.Components) > 0 {
			fromStr = strings.Join(source.Components, "/")
		}

		inviteID := fmt.Sprintf("inv-%s-%d", typeStr, sessionID)

		invite := &PendingInvite{
			ID:          inviteID,
			SessionID:   sessionID,
			Type:        typeStr,
			From:        fromStr,
			Destination: strings.Join(dest.Components, "/"),
			ReceivedAt:  time.Now(),
			ExpiresAt:   time.Now().Add(time.Duration(c.inviteTimeout) * time.Second),
			Session:     session,
		}

		c.mu.Lock()
		c.pendingInvites[inviteID] = invite
		c.mu.Unlock()

		log.Printf("Received invitation: %s (%s) from %s", inviteID, typeStr, fromStr)

		c.globalSSE.Broadcast(httputil.SSEEvent{
			Type: "invitation_received",
			Data: map[string]interface{}{
				"id":          inviteID,
				"type":        typeStr,
				"from":        fromStr,
				"destination": invite.Destination,
			},
		})
	}
}

// CleanupExpiredInvitations periodically removes expired invitations.
func (c *Client) CleanupExpiredInvitations(ctx context.Context) {
	ticker := time.NewTicker(config.InviteCleanupInterval)
	defer ticker.Stop()

	for {
		select {
		case <-ticker.C:
			c.mu.Lock()
			now := time.Now()
			for id, invite := range c.pendingInvites {
				if now.After(invite.ExpiresAt) {
					log.Printf("Invitation expired: %s", id)
					if c.app != nil && invite.Session != nil {
						c.app.DeleteSession(invite.Session)
					}
					delete(c.pendingInvites, id)

					c.globalSSE.Broadcast(httputil.SSEEvent{
						Type: "invitation_expired",
						Data: map[string]string{"id": id},
					})
				}
			}
			c.mu.Unlock()
		case <-c.stopCleanup:
			return
		case <-ctx.Done():
			return
		}
	}
}

func (c *Client) receiveMessages(session *JoinedSession) {
	for {
		select {
		case <-session.stopChan:
			return
		default:
		}

		timeout := uint32(config.GetMessageTimeoutMs)
		session.sessionMu.Lock()
		msg, err := session.Session.GetMessage(&timeout)
		session.sessionMu.Unlock()

		if err != nil {
			wrappedErr := slimErrors.WrapSLIMError(err)
			if slimErrors.IsTimeout(wrappedErr) {
				continue
			}
			if slimErrors.IsParticipantDisconnected(wrappedErr) {
				log.Printf("Session %s: %v", session.ID, err)
				continue
			}
			// Session closed or fatal error
			log.Printf("Session %s GetMessage error: %v", session.ID, err)
			c.mu.Lock()
			session.Status = "closed"
			c.mu.Unlock()

			c.globalSSE.Broadcast(httputil.SSEEvent{
				Type: "session_closed",
				Data: map[string]string{"id": session.ID},
			})
			return
		}

		sender := strings.Join(msg.Context.SourceName.Components, "/")
		message := message.Message{
			ID:        fmt.Sprintf("msg-%d", time.Now().UnixNano()),
			SessionID: session.ID,
			Direction: message.DirectionReceived,
			Sender:    sender,
			Text:      string(msg.Payload),
			Metadata:  msg.Context.Metadata,
			Timestamp: time.Now(),
		}

		c.messages.Add(session.ID, message)

		c.mu.Lock()
		session.MessageCount++
		c.mu.Unlock()

		c.sessionSSE.Broadcast(session.ID, httputil.SSEEvent{
			Type: "message",
			Data: formatMessageHTML(message),
		})
	}
}

// Close stops cleanup and releases resources.
func (c *Client) Close() {
	close(c.stopCleanup)
}

// formatMessageHTML formats a message as HTML for SSE.
func formatMessageHTML(msg message.Message) string {
	timestamp := msg.Timestamp.Format("15:04:05")
	class := "message"
	if msg.Direction == message.DirectionSent {
		class = "message sent"
	}
	return fmt.Sprintf(`<div class="%s"><span class="time">[%s]</span> <span class="sender">%s:</span> %s</div>`,
		class, timestamp, msg.Sender, msg.Text)
}
