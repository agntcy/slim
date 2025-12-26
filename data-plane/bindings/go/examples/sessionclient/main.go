// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Package main implements a web-based SLIM session client using Go and HTMX.
//
// This application provides a participant-focused interface for:
//   - Receiving session invitations from Session Manager
//   - Accepting or declining invitations
//   - Joining and participating in sessions
//   - Sending and receiving messages
//
// Usage:
//
//	# First start the SLIM server
//	task example:server
//
//	# Start the session manager (creates sessions)
//	task example:sessionmgr
//
//	# Then start the session client (joins sessions)
//	task example:sessionclient LOCAL=org/alice/app
//
//	# Open http://localhost:8082 in your browser
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"html/template"
	"log"
	"net/http"
	"strings"
	"sync"
	"time"

	slim "github.com/agntcy/slim/bindings/generated/slim_bindings"
	"github.com/agntcy/slim/bindings/go/examples/common"
)

// Configuration defaults
const (
	defaultPort          = 8082
	defaultSlimAddr      = "http://localhost:46357"
	defaultSecret        = "demo-shared-secret-min-32-chars!!"
	defaultLocalID       = "org/client/app"
	defaultInviteTimeout = 60 // seconds
)

var (
	slimEndpoint  string
	slimSecret    string
	localID       string
	serverPort    int
	inviteTimeout int
)

// PendingInvite represents an incoming session invitation
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

// TimeRemaining returns seconds until expiry
func (p *PendingInvite) TimeRemaining() int {
	remaining := int(time.Until(p.ExpiresAt).Seconds())
	if remaining < 0 {
		return 0
	}
	return remaining
}

// JoinedSession represents an accepted session
type JoinedSession struct {
	ID           string                       `json:"id"`
	SessionID    uint32                       `json:"sessionId"`
	Type         string                       `json:"type"` // "p2p" or "group"
	Destination  string                       `json:"destination"`
	JoinedAt     time.Time                    `json:"joinedAt"`
	MessageCount int                          `json:"messageCount"`
	Status       string                       `json:"status"` // "active", "closed"
	Session      *slim.BindingsSessionContext `json:"-"`
	sessionMu    sync.Mutex                   `json:"-"` // Mutex for thread-safe session operations
	stopChan     chan struct{}                `json:"-"`
}

// Message represents a chat message
type Message struct {
	ID        string            `json:"id"`
	SessionID string            `json:"sessionId"`
	Direction string            `json:"direction"` // "sent" or "received"
	Sender    string            `json:"sender"`
	Text      string            `json:"text"`
	Metadata  map[string]string `json:"metadata,omitempty"`
	Timestamp time.Time         `json:"timestamp"`
}

// SSEEvent represents an event to be sent via SSE
type SSEEvent struct {
	Type string      `json:"type"`
	Data interface{} `json:"data"`
}

// SessionClient holds all application state
type SessionClient struct {
	app     *slim.BindingsAdapter
	connID  uint64
	localID string

	pendingInvites map[string]*PendingInvite
	joinedSessions map[string]*JoinedSession
	messages       map[string][]Message
	mu             sync.RWMutex

	// SSE subscribers
	globalSSE  map[string]chan SSEEvent
	sessionSSE map[string]map[string]chan SSEEvent
	sseMu      sync.RWMutex

	// Stop channel for cleanup goroutine
	stopCleanup chan struct{}
}

var (
	client    *SessionClient
	templates *template.Template
)

func main() {
	// Parse command-line flags
	port := flag.Int("port", defaultPort, "HTTP server port")
	flag.StringVar(&slimEndpoint, "slim", defaultSlimAddr, "SLIM server endpoint")
	flag.StringVar(&slimSecret, "secret", defaultSecret, "SLIM shared secret")
	flag.StringVar(&localID, "local", defaultLocalID, "Local SLIM identity (org/namespace/app)")
	flag.IntVar(&inviteTimeout, "timeout", defaultInviteTimeout, "Invitation timeout in seconds")
	flag.Parse()

	serverPort = *port
	httpAddr := fmt.Sprintf(":%d", *port)

	// Initialize SLIM
	var err error
	client, err = initSLIM(localID, slimEndpoint, slimSecret)
	if err != nil {
		log.Printf("Warning: SLIM connection failed: %v", err)
		log.Printf("Running in demo mode without SLIM connectivity")
		client = &SessionClient{
			localID:        localID,
			pendingInvites: make(map[string]*PendingInvite),
			joinedSessions: make(map[string]*JoinedSession),
			messages:       make(map[string][]Message),
			globalSSE:      make(map[string]chan SSEEvent),
			sessionSSE:     make(map[string]map[string]chan SSEEvent),
			stopCleanup:    make(chan struct{}),
		}
	}

	// Start cleanup goroutine for expired invitations
	go client.cleanupExpiredInvitations()

	// Parse templates
	templates, err = template.New("").Funcs(template.FuncMap{
		"json": func(v interface{}) string {
			b, _ := json.Marshal(v)
			return string(b)
		},
		"formatTime": func(t time.Time) string {
			return t.Format("15:04:05")
		},
		"formatDate": func(t time.Time) string {
			return t.Format("2006-01-02 15:04:05")
		},
	}).Parse(templatesHTML)
	if err != nil {
		log.Fatalf("Failed to parse templates: %v", err)
	}

	// Set up routes
	http.HandleFunc("/", handleHome)
	http.HandleFunc("/login", handleLogin)
	http.HandleFunc("/logout", handleLogout)
	http.HandleFunc("/invitations", handleInvitations)
	http.HandleFunc("/invitation/", handleInvitationRoutes)
	http.HandleFunc("/sessions", handleSessions)
	http.HandleFunc("/session/", handleSessionRoutes)
	http.HandleFunc("/events", handleGlobalSSE)

	log.Printf("Starting SLIM Session Client on http://localhost%s", httpAddr)
	log.Printf("SLIM server: %s, Local ID: %s", slimEndpoint, localID)
	log.Printf("Invitation timeout: %d seconds", inviteTimeout)
	if err := http.ListenAndServe(httpAddr, nil); err != nil {
		log.Fatalf("Server failed: %v", err)
	}
}

// initSLIM initializes the SLIM connection
func initSLIM(localID, serverAddr, secret string) (*SessionClient, error) {
	app, connID, err := common.CreateAndConnectApp(localID, serverAddr, secret)
	if err != nil {
		return nil, err
	}

	c := &SessionClient{
		app:            app,
		connID:         connID,
		localID:        localID,
		pendingInvites: make(map[string]*PendingInvite),
		joinedSessions: make(map[string]*JoinedSession),
		messages:       make(map[string][]Message),
		globalSSE:      make(map[string]chan SSEEvent),
		sessionSSE:     make(map[string]map[string]chan SSEEvent),
		stopCleanup:    make(chan struct{}),
	}

	// Start listening for incoming session invitations
	go c.listenForInvitations()

	return c, nil
}

// usernameCookieName returns the port-specific cookie name
func usernameCookieName() string {
	return fmt.Sprintf("sessionclient_user_%d", serverPort)
}

// getUsername returns the username from cookie
func getUsername(r *http.Request) string {
	cookie, err := r.Cookie(usernameCookieName())
	if err != nil {
		return ""
	}
	return cookie.Value
}

// handleHome serves the home page
func handleHome(w http.ResponseWriter, r *http.Request) {
	if r.URL.Path != "/" {
		http.NotFound(w, r)
		return
	}

	username := getUsername(r)
	if username == "" {
		templates.ExecuteTemplate(w, "login", nil)
		return
	}

	client.mu.RLock()
	invites := make([]*PendingInvite, 0, len(client.pendingInvites))
	for _, inv := range client.pendingInvites {
		invites = append(invites, inv)
	}
	sessions := make([]*JoinedSession, 0, len(client.joinedSessions))
	for _, s := range client.joinedSessions {
		sessions = append(sessions, s)
	}
	client.mu.RUnlock()

	data := map[string]interface{}{
		"Username":    username,
		"Invitations": invites,
		"Sessions":    sessions,
		"LocalID":     client.localID,
	}
	templates.ExecuteTemplate(w, "home", data)
}

// handleLogin sets the username
func handleLogin(w http.ResponseWriter, r *http.Request) {
	if r.Method != "POST" {
		http.Redirect(w, r, "/", http.StatusSeeOther)
		return
	}

	username := strings.TrimSpace(r.FormValue("username"))
	if username == "" {
		http.Error(w, "Username required", http.StatusBadRequest)
		return
	}

	http.SetCookie(w, &http.Cookie{
		Name:   usernameCookieName(),
		Value:  username,
		Path:   "/",
		MaxAge: 86400 * 7,
	})

	http.Redirect(w, r, "/", http.StatusSeeOther)
}

// handleLogout clears the username
func handleLogout(w http.ResponseWriter, r *http.Request) {
	http.SetCookie(w, &http.Cookie{
		Name:   usernameCookieName(),
		Value:  "",
		Path:   "/",
		MaxAge: -1,
	})
	http.Redirect(w, r, "/", http.StatusSeeOther)
}

// handleInvitations returns the invitations list partial
func handleInvitations(w http.ResponseWriter, r *http.Request) {
	client.mu.RLock()
	invites := make([]*PendingInvite, 0, len(client.pendingInvites))
	for _, inv := range client.pendingInvites {
		invites = append(invites, inv)
	}
	client.mu.RUnlock()

	templates.ExecuteTemplate(w, "invitations_list", invites)
}

// handleInvitationRoutes routes invitation-specific requests
func handleInvitationRoutes(w http.ResponseWriter, r *http.Request) {
	path := strings.TrimPrefix(r.URL.Path, "/invitation/")
	parts := strings.Split(path, "/")
	inviteID := parts[0]

	if inviteID == "" || len(parts) < 2 {
		http.NotFound(w, r)
		return
	}

	action := parts[1]
	switch action {
	case "accept":
		handleAcceptInvitation(w, r, inviteID)
	case "decline":
		handleDeclineInvitation(w, r, inviteID)
	default:
		http.NotFound(w, r)
	}
}

// handleAcceptInvitation accepts a pending invitation
func handleAcceptInvitation(w http.ResponseWriter, r *http.Request, inviteID string) {
	if r.Method != "POST" {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	if err := client.acceptInvitation(inviteID); err != nil {
		log.Printf("Failed to accept invitation %s: %v", inviteID, err)
		http.Error(w, "Failed to accept: "+err.Error(), http.StatusInternalServerError)
		return
	}

	log.Printf("Accepted invitation: %s", inviteID)

	// Return updated invitations list
	client.mu.RLock()
	invites := make([]*PendingInvite, 0, len(client.pendingInvites))
	for _, inv := range client.pendingInvites {
		invites = append(invites, inv)
	}
	client.mu.RUnlock()

	templates.ExecuteTemplate(w, "invitations_list", invites)
}

// handleDeclineInvitation declines a pending invitation
func handleDeclineInvitation(w http.ResponseWriter, r *http.Request, inviteID string) {
	if r.Method != "POST" {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	if err := client.declineInvitation(inviteID); err != nil {
		log.Printf("Failed to decline invitation %s: %v", inviteID, err)
		http.Error(w, "Failed to decline: "+err.Error(), http.StatusInternalServerError)
		return
	}

	log.Printf("Declined invitation: %s", inviteID)

	// Return updated invitations list
	client.mu.RLock()
	invites := make([]*PendingInvite, 0, len(client.pendingInvites))
	for _, inv := range client.pendingInvites {
		invites = append(invites, inv)
	}
	client.mu.RUnlock()

	templates.ExecuteTemplate(w, "invitations_list", invites)
}

// handleSessions returns the sessions list partial
func handleSessions(w http.ResponseWriter, r *http.Request) {
	client.mu.RLock()
	sessions := make([]*JoinedSession, 0, len(client.joinedSessions))
	for _, s := range client.joinedSessions {
		sessions = append(sessions, s)
	}
	client.mu.RUnlock()

	templates.ExecuteTemplate(w, "sessions_list", sessions)
}

// handleSessionRoutes routes session-specific requests
func handleSessionRoutes(w http.ResponseWriter, r *http.Request) {
	path := strings.TrimPrefix(r.URL.Path, "/session/")
	parts := strings.Split(path, "/")
	sessionID := parts[0]

	if sessionID == "" {
		http.Redirect(w, r, "/", http.StatusSeeOther)
		return
	}

	if len(parts) == 1 {
		handleSessionDetail(w, r, sessionID)
		return
	}

	action := parts[1]
	switch action {
	case "send":
		handleSendMessage(w, r, sessionID)
	case "leave":
		handleLeaveSession(w, r, sessionID)
	case "events":
		handleSessionSSE(w, r, sessionID)
	default:
		http.NotFound(w, r)
	}
}

// handleSessionDetail shows session detail page
func handleSessionDetail(w http.ResponseWriter, r *http.Request, sessionID string) {
	username := getUsername(r)
	if username == "" {
		http.Redirect(w, r, "/", http.StatusSeeOther)
		return
	}

	client.mu.RLock()
	session, exists := client.joinedSessions[sessionID]
	messages := client.messages[sessionID]
	client.mu.RUnlock()

	if !exists {
		http.Error(w, "Session not found", http.StatusNotFound)
		return
	}

	data := map[string]interface{}{
		"Username": username,
		"Session":  session,
		"Messages": messages,
		"LocalID":  client.localID,
	}
	templates.ExecuteTemplate(w, "session_detail", data)
}

// handleSendMessage sends a message in a session
func handleSendMessage(w http.ResponseWriter, r *http.Request, sessionID string) {
	if r.Method != "POST" {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	text := strings.TrimSpace(r.FormValue("message"))
	if text == "" {
		w.WriteHeader(http.StatusOK)
		return
	}

	if err := client.sendMessage(sessionID, text); err != nil {
		http.Error(w, "Failed to send: "+err.Error(), http.StatusInternalServerError)
		return
	}

	w.WriteHeader(http.StatusOK)
}

// handleLeaveSession leaves a session
func handleLeaveSession(w http.ResponseWriter, r *http.Request, sessionID string) {
	if r.Method != "POST" {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	if err := client.leaveSession(sessionID); err != nil {
		http.Error(w, "Failed to leave: "+err.Error(), http.StatusInternalServerError)
		return
	}

	log.Printf("Left session: %s", sessionID)

	// Redirect to home
	if r.Header.Get("HX-Request") == "true" {
		w.Header().Set("HX-Redirect", "/")
		w.WriteHeader(http.StatusOK)
		return
	}

	http.Redirect(w, r, "/", http.StatusSeeOther)
}

// handleGlobalSSE handles global SSE connections
func handleGlobalSSE(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	w.Header().Set("Connection", "keep-alive")
	w.Header().Set("X-Accel-Buffering", "no")

	flusher, ok := w.(http.Flusher)
	if !ok {
		http.Error(w, "SSE not supported", http.StatusInternalServerError)
		return
	}

	subID := fmt.Sprintf("sub-%d", time.Now().UnixNano())
	events := make(chan SSEEvent, 100)

	client.sseMu.Lock()
	client.globalSSE[subID] = events
	client.sseMu.Unlock()

	defer func() {
		client.sseMu.Lock()
		delete(client.globalSSE, subID)
		client.sseMu.Unlock()
		close(events)
	}()

	fmt.Fprintf(w, "event: ping\ndata: connected\n\n")
	flusher.Flush()

	for {
		select {
		case event := <-events:
			data, _ := json.Marshal(event.Data)
			fmt.Fprintf(w, "event: %s\ndata: %s\n\n", event.Type, string(data))
			flusher.Flush()
		case <-r.Context().Done():
			return
		}
	}
}

// handleSessionSSE handles session-specific SSE connections
func handleSessionSSE(w http.ResponseWriter, r *http.Request, sessionID string) {
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	w.Header().Set("Connection", "keep-alive")
	w.Header().Set("X-Accel-Buffering", "no")

	flusher, ok := w.(http.Flusher)
	if !ok {
		http.Error(w, "SSE not supported", http.StatusInternalServerError)
		return
	}

	subID := fmt.Sprintf("sub-%d", time.Now().UnixNano())
	events := make(chan SSEEvent, 100)

	client.sseMu.Lock()
	if client.sessionSSE[sessionID] == nil {
		client.sessionSSE[sessionID] = make(map[string]chan SSEEvent)
	}
	client.sessionSSE[sessionID][subID] = events
	client.sseMu.Unlock()

	defer func() {
		client.sseMu.Lock()
		delete(client.sessionSSE[sessionID], subID)
		client.sseMu.Unlock()
		close(events)
	}()

	// Send existing messages first
	client.mu.RLock()
	messages := client.messages[sessionID]
	client.mu.RUnlock()

	for _, msg := range messages {
		html := formatMessageHTML(msg)
		fmt.Fprintf(w, "event: message\ndata: %s\n\n", html)
	}
	flusher.Flush()

	for {
		select {
		case event := <-events:
			if event.Type == "message" {
				fmt.Fprintf(w, "event: message\ndata: %s\n\n", event.Data)
			} else {
				data, _ := json.Marshal(event.Data)
				fmt.Fprintf(w, "event: %s\ndata: %s\n\n", event.Type, string(data))
			}
			flusher.Flush()
		case <-r.Context().Done():
			return
		}
	}
}

// SessionClient methods

func (c *SessionClient) listenForInvitations() {
	if c.app == nil {
		return
	}

	for {
		timeout := uint32(5000)
		session, err := c.app.ListenForSession(&timeout)
		if err != nil {
			errStr := err.Error()
			if strings.Contains(errStr, "timeout") || strings.Contains(errStr, "timed out") {
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
			ExpiresAt:   time.Now().Add(time.Duration(inviteTimeout) * time.Second),
			Session:     session,
		}

		c.mu.Lock()
		c.pendingInvites[inviteID] = invite
		c.mu.Unlock()

		log.Printf("Received invitation: %s (%s) from %s", inviteID, typeStr, fromStr)

		c.broadcastGlobalEvent(SSEEvent{
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

func (c *SessionClient) cleanupExpiredInvitations() {
	ticker := time.NewTicker(5 * time.Second)
	defer ticker.Stop()

	for {
		select {
		case <-ticker.C:
			c.mu.Lock()
			now := time.Now()
			for id, invite := range c.pendingInvites {
				if now.After(invite.ExpiresAt) {
					log.Printf("Invitation expired: %s", id)
					// Delete the session to clean up
					if c.app != nil && invite.Session != nil {
						c.app.DeleteSession(invite.Session)
					}
					delete(c.pendingInvites, id)

					c.broadcastGlobalEvent(SSEEvent{
						Type: "invitation_expired",
						Data: map[string]string{"id": id},
					})
				}
			}
			c.mu.Unlock()
		case <-c.stopCleanup:
			return
		}
	}
}

func (c *SessionClient) acceptInvitation(inviteID string) error {
	c.mu.Lock()
	invite, exists := c.pendingInvites[inviteID]
	if !exists {
		c.mu.Unlock()
		return fmt.Errorf("invitation not found or expired")
	}
	delete(c.pendingInvites, inviteID)
	c.mu.Unlock()

	// Check if expired
	if time.Now().After(invite.ExpiresAt) {
		if c.app != nil && invite.Session != nil {
			c.app.DeleteSession(invite.Session)
		}
		return fmt.Errorf("invitation has expired")
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
	c.messages[sessionID] = []Message{}
	c.mu.Unlock()

	// Start message receiver after a brief delay to let session establish
	if invite.Session != nil {
		time.Sleep(100 * time.Millisecond)
		go c.receiveMessages(joined)
	}

	c.broadcastGlobalEvent(SSEEvent{
		Type: "session_joined",
		Data: map[string]interface{}{
			"id":          sessionID,
			"type":        joined.Type,
			"destination": joined.Destination,
		},
	})

	return nil
}

func (c *SessionClient) declineInvitation(inviteID string) error {
	c.mu.Lock()
	invite, exists := c.pendingInvites[inviteID]
	if !exists {
		c.mu.Unlock()
		return fmt.Errorf("invitation not found")
	}
	delete(c.pendingInvites, inviteID)
	c.mu.Unlock()

	// Delete the session to clean up
	if c.app != nil && invite.Session != nil {
		c.app.DeleteSession(invite.Session)
	}

	c.broadcastGlobalEvent(SSEEvent{
		Type: "invitation_expired",
		Data: map[string]string{"id": inviteID},
	})

	return nil
}

func (c *SessionClient) leaveSession(sessionID string) error {
	c.mu.Lock()
	session, exists := c.joinedSessions[sessionID]
	if !exists {
		c.mu.Unlock()
		return fmt.Errorf("session not found")
	}
	delete(c.joinedSessions, sessionID)
	delete(c.messages, sessionID)
	c.mu.Unlock()

	// Stop message receiver
	if session.stopChan != nil {
		close(session.stopChan)
	}

	// Delete SLIM session
	if c.app != nil && session.Session != nil {
		c.app.DeleteSession(session.Session)
	}

	c.broadcastGlobalEvent(SSEEvent{
		Type: "session_left",
		Data: map[string]string{"id": sessionID},
	})

	return nil
}

func (c *SessionClient) sendMessage(sessionID, text string) error {
	c.mu.RLock()
	session, exists := c.joinedSessions[sessionID]
	c.mu.RUnlock()

	if !exists {
		return fmt.Errorf("session not found")
	}

	if c.app != nil && session.Session != nil {
		session.sessionMu.Lock()
		err := session.Session.Publish([]byte(text), nil, nil)
		session.sessionMu.Unlock()
		if err != nil {
			return fmt.Errorf("failed to publish: %w", err)
		}
	}

	msg := Message{
		ID:        fmt.Sprintf("msg-%d", time.Now().UnixNano()),
		SessionID: sessionID,
		Direction: "sent",
		Sender:    "you",
		Text:      text,
		Timestamp: time.Now(),
	}

	c.mu.Lock()
	c.messages[sessionID] = append(c.messages[sessionID], msg)
	session.MessageCount++
	c.mu.Unlock()

	c.broadcastSessionEvent(sessionID, SSEEvent{
		Type: "message",
		Data: formatMessageHTML(msg),
	})

	return nil
}

func (c *SessionClient) receiveMessages(session *JoinedSession) {
	for {
		select {
		case <-session.stopChan:
			return
		default:
		}

		timeout := uint32(1000)
		session.sessionMu.Lock()
		msg, err := session.Session.GetMessage(&timeout)
		session.sessionMu.Unlock()
		if err != nil {
			errStr := err.Error()
			if strings.Contains(errStr, "timeout") || strings.Contains(errStr, "timed out") {
				continue
			}
			// Participant disconnected is informational, not fatal
			if strings.Contains(errStr, "participant disconnected") {
				log.Printf("Session %s: %v", session.ID, err)
				continue
			}
			// Session closed or fatal error
			log.Printf("Session %s GetMessage error: %v", session.ID, err)
			c.mu.Lock()
			session.Status = "closed"
			c.mu.Unlock()

			c.broadcastGlobalEvent(SSEEvent{
				Type: "session_closed",
				Data: map[string]string{"id": session.ID},
			})
			return
		}

		sender := strings.Join(msg.Context.SourceName.Components, "/")
		message := Message{
			ID:        fmt.Sprintf("msg-%d", time.Now().UnixNano()),
			SessionID: session.ID,
			Direction: "received",
			Sender:    sender,
			Text:      string(msg.Payload),
			Metadata:  msg.Context.Metadata,
			Timestamp: time.Now(),
		}

		c.mu.Lock()
		c.messages[session.ID] = append(c.messages[session.ID], message)
		session.MessageCount++
		c.mu.Unlock()

		c.broadcastSessionEvent(session.ID, SSEEvent{
			Type: "message",
			Data: formatMessageHTML(message),
		})
	}
}

func (c *SessionClient) broadcastGlobalEvent(event SSEEvent) {
	c.sseMu.RLock()
	defer c.sseMu.RUnlock()

	for _, ch := range c.globalSSE {
		select {
		case ch <- event:
		default:
		}
	}
}

func (c *SessionClient) broadcastSessionEvent(sessionID string, event SSEEvent) {
	c.sseMu.RLock()
	defer c.sseMu.RUnlock()

	if subs, ok := c.sessionSSE[sessionID]; ok {
		for _, ch := range subs {
			select {
			case ch <- event:
			default:
			}
		}
	}
}

func formatMessageHTML(msg Message) string {
	timestamp := msg.Timestamp.Format("15:04:05")
	class := "message"
	if msg.Direction == "sent" {
		class = "message sent"
	}
	text := template.HTMLEscapeString(msg.Text)
	sender := template.HTMLEscapeString(msg.Sender)
	return fmt.Sprintf(`<div class="%s"><span class="time">[%s]</span> <span class="sender">%s:</span> %s</div>`,
		class, timestamp, sender, text)
}

// Embedded HTML templates
const templatesHTML = `
{{define "login"}}
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Login - SLIM Session Client</title>
    <style>
        * { box-sizing: border-box; margin: 0; padding: 0; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #1a1a2e; color: #eee; min-height: 100vh; display: flex; align-items: center; justify-content: center; }
        .login-box { background: #16213e; border-radius: 8px; padding: 40px; width: 100%; max-width: 400px; }
        h1 { color: #2ecc71; margin-bottom: 10px; text-align: center; }
        p { color: #888; margin-bottom: 30px; text-align: center; }
        input[type="text"] { width: 100%; padding: 12px; border: 1px solid #333; border-radius: 4px; background: #0f0f23; color: #fff; font-size: 16px; margin-bottom: 15px; }
        input[type="text"]:focus { outline: none; border-color: #2ecc71; }
        button { width: 100%; background: #2ecc71; color: #000; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; font-size: 16px; font-weight: bold; }
        button:hover { background: #27ae60; }
    </style>
</head>
<body>
    <div class="login-box">
        <h1>SLIM Session Client</h1>
        <p>Enter your username to continue</p>
        <form action="/login" method="POST">
            <input type="text" name="username" placeholder="Username..." required autofocus>
            <button type="submit">Continue</button>
        </form>
    </div>
</body>
</html>
{{end}}

{{define "home"}}
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>SLIM Session Client</title>
    <script src="https://unpkg.com/htmx.org@2.0.4"></script>
    <script src="https://unpkg.com/htmx-ext-sse@2.2.2/sse.js"></script>
    <style>
        * { box-sizing: border-box; margin: 0; padding: 0; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #1a1a2e; color: #eee; min-height: 100vh; }
        .container { max-width: 900px; margin: 0 auto; padding: 20px; }
        .header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px; padding-bottom: 15px; border-bottom: 1px solid #333; }
        h1 { color: #2ecc71; }
        .user-info { display: flex; align-items: center; gap: 15px; }
        .user-info span { color: #2ecc71; }
        .user-info a { color: #888; text-decoration: none; font-size: 14px; }
        .user-info a:hover { color: #fff; }
        .local-id { color: #666; font-size: 12px; }
        h2 { color: #888; font-size: 14px; text-transform: uppercase; margin: 20px 0 10px; display: flex; align-items: center; gap: 10px; }
        .count { background: #333; padding: 2px 8px; border-radius: 10px; font-size: 12px; }
        .card { background: #16213e; border-radius: 8px; padding: 20px; margin-bottom: 20px; }
        .invite-list, .session-list { display: grid; gap: 15px; }
        .invite-card { display: flex; justify-content: space-between; align-items: center; padding: 15px; background: #0f0f23; border-radius: 4px; border-left: 4px solid #f39c12; }
        .session-card { display: flex; justify-content: space-between; align-items: center; padding: 15px; background: #0f0f23; border-radius: 4px; border-left: 4px solid #2ecc71; }
        .session-card.closed { opacity: 0.6; border-left-color: #666; }
        .invite-info, .session-info { flex: 1; }
        .invite-type, .session-id { font-weight: bold; color: #2ecc71; font-family: monospace; }
        .invite-meta, .session-meta { color: #888; font-size: 14px; margin-top: 5px; }
        .invite-meta span, .session-meta span { margin-right: 15px; }
        .badge { display: inline-block; padding: 2px 8px; border-radius: 3px; font-size: 12px; text-transform: uppercase; }
        .badge.p2p { background: #00d4ff; color: #000; }
        .badge.group { background: #9b59b6; color: #fff; }
        .badge.active { background: #2ecc71; color: #000; }
        .badge.closed { background: #666; color: #fff; }
        .time-remaining { color: #f39c12; font-weight: bold; }
        .invite-actions, .session-actions { display: flex; gap: 10px; }
        .invite-actions button, .session-actions a, .session-actions button { padding: 8px 16px; font-size: 14px; text-decoration: none; border-radius: 4px; border: none; cursor: pointer; }
        .btn-accept { background: #2ecc71; color: #000; font-weight: bold; }
        .btn-accept:hover { background: #27ae60; }
        .btn-decline { background: #e74c3c; color: #fff; }
        .btn-decline:hover { background: #c0392b; }
        .btn-open { background: #3498db; color: #fff; }
        .btn-open:hover { background: #2980b9; }
        .btn-leave { background: #e74c3c; color: #fff; }
        .btn-leave:hover { background: #c0392b; }
        .empty { color: #666; text-align: center; padding: 40px; }
        .waiting { display: flex; align-items: center; gap: 10px; color: #888; }
        .waiting-dot { width: 8px; height: 8px; background: #2ecc71; border-radius: 50%; animation: pulse 1.5s infinite; }
        @keyframes pulse { 0%, 100% { opacity: 0.3; } 50% { opacity: 1; } }
    </style>
</head>
<body hx-ext="sse" sse-connect="/events">
    <div class="container">
        <div class="header">
            <div>
                <h1>SLIM Session Client</h1>
                <div class="local-id">Local ID: {{.LocalID}}</div>
            </div>
            <div class="user-info">
                <span>{{.Username}}</span>
                <a href="/logout">Logout</a>
            </div>
        </div>

        <div class="card">
            <h2>Pending Invitations <span class="count" id="invite-count">{{len .Invitations}}</span></h2>
            <div id="invitations" hx-get="/invitations" hx-trigger="sse:invitation_received, sse:invitation_expired">
                {{template "invitations_list" .Invitations}}
            </div>
        </div>

        <div class="card">
            <h2>Joined Sessions <span class="count" id="session-count">{{len .Sessions}}</span></h2>
            <div id="sessions" hx-get="/sessions" hx-trigger="sse:session_joined, sse:session_left, sse:session_closed">
                {{template "sessions_list" .Sessions}}
            </div>
        </div>
    </div>
</body>
</html>
{{end}}

{{define "invitations_list"}}
{{if .}}
<div class="invite-list">
    {{range .}}
    <div class="invite-card" id="invite-{{.ID}}">
        <div class="invite-info">
            <div>
                <span class="badge {{.Type}}">{{.Type}}</span>
                <span class="time-remaining">{{.TimeRemaining}}s remaining</span>
            </div>
            <div class="invite-type">{{.ID}}</div>
            <div class="invite-meta">
                {{if .From}}<span>From: {{.From}}</span>{{end}}
                <span>To: {{.Destination}}</span>
            </div>
        </div>
        <div class="invite-actions">
            <button class="btn-accept" hx-post="/invitation/{{.ID}}/accept" hx-target="#invitations" hx-swap="innerHTML">Accept</button>
            <button class="btn-decline" hx-post="/invitation/{{.ID}}/decline" hx-target="#invitations" hx-swap="innerHTML">Decline</button>
        </div>
    </div>
    {{end}}
</div>
{{else}}
<div class="empty">
    <div class="waiting">
        <div class="waiting-dot"></div>
        Waiting for session invitations...
    </div>
</div>
{{end}}
{{end}}

{{define "sessions_list"}}
{{if .}}
<div class="session-list">
    {{range .}}
    <div class="session-card {{.Status}}" id="session-{{.ID}}">
        <div class="session-info">
            <div>
                <span class="badge {{.Type}}">{{.Type}}</span>
                <span class="badge {{.Status}}">{{.Status}}</span>
            </div>
            <div class="session-id">{{.ID}}</div>
            <div class="session-meta">
                <span>{{.Destination}}</span>
                <span>Messages: {{.MessageCount}}</span>
            </div>
        </div>
        <div class="session-actions">
            <a href="/session/{{.ID}}" class="btn-open">Open</a>
            <button class="btn-leave" hx-post="/session/{{.ID}}/leave" hx-confirm="Leave this session?">Leave</button>
        </div>
    </div>
    {{end}}
</div>
{{else}}
<p class="empty">No joined sessions. Accept an invitation to join!</p>
{{end}}
{{end}}

{{define "session_detail"}}
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Session {{.Session.ID}} - SLIM Session Client</title>
    <script src="https://unpkg.com/htmx.org@2.0.4"></script>
    <script src="https://unpkg.com/htmx-ext-sse@2.2.2/sse.js"></script>
    <style>
        * { box-sizing: border-box; margin: 0; padding: 0; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #1a1a2e; color: #eee; min-height: 100vh; display: flex; flex-direction: column; }
        .header { background: #16213e; padding: 15px 20px; display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid #333; }
        .header h1 { font-size: 18px; color: #2ecc71; }
        .header-left { display: flex; align-items: center; gap: 20px; }
        .header a { color: #888; text-decoration: none; }
        .header a:hover { color: #fff; }
        .badge { display: inline-block; padding: 2px 8px; border-radius: 3px; font-size: 12px; text-transform: uppercase; margin-left: 10px; }
        .badge.p2p { background: #00d4ff; color: #000; }
        .badge.group { background: #9b59b6; color: #fff; }
        .badge.active { background: #2ecc71; color: #000; }
        .main { display: flex; flex: 1; overflow: hidden; }
        .sidebar { width: 250px; background: #16213e; border-right: 1px solid #333; padding: 15px; overflow-y: auto; }
        .sidebar h3 { color: #888; font-size: 12px; text-transform: uppercase; margin-bottom: 10px; }
        .info-item { margin-bottom: 15px; }
        .info-label { color: #666; font-size: 12px; }
        .info-value { color: #fff; font-family: monospace; word-break: break-all; }
        .chat-area { flex: 1; display: flex; flex-direction: column; }
        .messages { flex: 1; overflow-y: auto; padding: 20px; }
        .message { padding: 8px 0; border-bottom: 1px solid #222; }
        .message .time { color: #666; }
        .message .sender { color: #2ecc71; font-weight: bold; }
        .message.sent .sender { color: #3498db; }
        .input-area { background: #16213e; padding: 15px 20px; border-top: 1px solid #333; }
        .input-row { display: flex; gap: 10px; }
        input[type="text"] { flex: 1; padding: 12px; border: 1px solid #333; border-radius: 4px; background: #0f0f23; color: #fff; font-size: 16px; }
        input[type="text"]:focus { outline: none; border-color: #2ecc71; }
        button { background: #2ecc71; color: #000; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; font-size: 16px; font-weight: bold; }
        button:hover { background: #27ae60; }
        .btn-leave { background: #e74c3c; color: #fff; }
        .btn-leave:hover { background: #c0392b; }
    </style>
</head>
<body>
    <div class="header">
        <div class="header-left">
            <a href="/">&larr; Back</a>
            <h1>Session: {{.Session.ID}}</h1>
            <span class="badge {{.Session.Type}}">{{.Session.Type}}</span>
            <span class="badge {{.Session.Status}}">{{.Session.Status}}</span>
        </div>
        <button class="btn-leave" hx-post="/session/{{.Session.ID}}/leave" hx-confirm="Leave this session?">Leave</button>
    </div>

    <div class="main">
        <div class="sidebar">
            <h3>Session Info</h3>
            <div class="info-item">
                <div class="info-label">Session ID</div>
                <div class="info-value">{{.Session.SessionID}}</div>
            </div>
            <div class="info-item">
                <div class="info-label">Destination</div>
                <div class="info-value">{{.Session.Destination}}</div>
            </div>
            <div class="info-item">
                <div class="info-label">Joined</div>
                <div class="info-value">{{formatDate .Session.JoinedAt}}</div>
            </div>
            <div class="info-item">
                <div class="info-label">Messages</div>
                <div class="info-value">{{.Session.MessageCount}}</div>
            </div>
        </div>

        <div class="chat-area">
            <div class="messages" id="messages" hx-ext="sse" sse-connect="/session/{{.Session.ID}}/events" sse-swap="message" hx-swap="beforeend">
            </div>

            <div class="input-area">
                <form hx-post="/session/{{.Session.ID}}/send" hx-swap="none" hx-on::after-request="this.reset()">
                    <div class="input-row">
                        <input type="text" name="message" placeholder="Type a message..." autocomplete="off" autofocus>
                        <button type="submit">Send</button>
                    </div>
                </form>
            </div>
        </div>
    </div>

    <script>
        const messages = document.getElementById('messages');
        const observer = new MutationObserver(() => {
            messages.scrollTop = messages.scrollHeight;
        });
        observer.observe(messages, { childList: true });
    </script>
</body>
</html>
{{end}}
`
