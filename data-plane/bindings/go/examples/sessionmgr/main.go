// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Package main implements a web-based SLIM session manager using Go and HTMX.
//
// This application provides a dashboard for managing SLIM sessions including:
//   - Creating Point-to-Point and Group sessions
//   - Viewing session details (ID, type, destination, participants, messages)
//   - Inviting and removing participants from group sessions
//   - Sending and receiving messages per session
//   - Deleting/discarding sessions
//
// Usage:
//
//	# First start the SLIM server
//	task example:server
//
//	# Then start the session manager
//	task example:sessionmgr
//
//	# Open http://localhost:8081 in your browser
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
	defaultPort     = 8081
	defaultSlimAddr = "http://localhost:46357"
	defaultSecret   = "demo-shared-secret-min-32-chars!!"
	defaultLocalID  = "org/sessionmgr/app"
)

var (
	slimEndpoint string
	slimSecret   string
	localID      string
	serverPort   int
)

// SessionInfo represents an active SLIM session with its metadata
type SessionInfo struct {
	ID           string                          `json:"id"`
	SessionID    uint32                          `json:"sessionId"`
	Type         string                          `json:"type"` // "p2p" or "group"
	Destination  string                          `json:"destination"`
	IsInitiator  bool                            `json:"isInitiator"`
	Participants []string                        `json:"participants"`
	MessageCount int                             `json:"messageCount"`
	CreatedAt    time.Time                       `json:"createdAt"`
	LastActivity time.Time                       `json:"lastActivity"`
	Status       string                          `json:"status"` // "active", "closed", "error"
	EnableMLS    bool                            `json:"enableMls"`
	Session      *slim.BindingsSessionContext    `json:"-"`
	sessionMu    sync.Mutex                      `json:"-"` // Mutex for thread-safe session operations
	stopChan     chan struct{}                   `json:"-"`
}

// Message represents a chat message in a session
type Message struct {
	ID        string            `json:"id"`
	SessionID string            `json:"sessionId"`
	Direction string            `json:"direction"` // "sent" or "received"
	Sender    string            `json:"sender"`
	Text      string            `json:"text"`
	Metadata  map[string]string `json:"metadata,omitempty"`
	Timestamp time.Time         `json:"timestamp"`
}

// KnownClient represents a discovered/known client for invite suggestions
type KnownClient struct {
	Name     string    `json:"name"`
	LastSeen time.Time `json:"lastSeen"`
	Source   string    `json:"source"` // "message", "invite", "manual"
}

// SSEEvent represents an event to be sent via SSE
type SSEEvent struct {
	Type string      `json:"type"`
	Data interface{} `json:"data"`
}

// SessionManager holds all application state
type SessionManager struct {
	app      *slim.BindingsAdapter
	connID   uint64
	localID  string
	sessions map[string]*SessionInfo
	messages map[string][]Message
	clients  map[string]*KnownClient
	mu       sync.RWMutex

	// SSE subscribers
	globalSSE   map[string]chan SSEEvent
	sessionSSE  map[string]map[string]chan SSEEvent // sessionID -> subscriberID -> channel
	sseMu       sync.RWMutex
}

var (
	manager   *SessionManager
	templates *template.Template
)

func main() {
	// Parse command-line flags
	port := flag.Int("port", defaultPort, "HTTP server port")
	flag.StringVar(&slimEndpoint, "slim", defaultSlimAddr, "SLIM server endpoint")
	flag.StringVar(&slimSecret, "secret", defaultSecret, "SLIM shared secret")
	flag.StringVar(&localID, "local", defaultLocalID, "Local SLIM identity (org/namespace/app)")
	flag.Parse()

	serverPort = *port
	httpAddr := fmt.Sprintf(":%d", *port)

	// Initialize SLIM
	var err error
	manager, err = initSLIM(localID, slimEndpoint, slimSecret)
	if err != nil {
		log.Printf("Warning: SLIM connection failed: %v", err)
		log.Printf("Running in demo mode without SLIM connectivity")
		manager = &SessionManager{
			localID:    localID,
			sessions:   make(map[string]*SessionInfo),
			messages:   make(map[string][]Message),
			clients:    make(map[string]*KnownClient),
			globalSSE:  make(map[string]chan SSEEvent),
			sessionSSE: make(map[string]map[string]chan SSEEvent),
		}
	}

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
	http.HandleFunc("/sessions", handleSessions)
	http.HandleFunc("/session/", handleSessionRoutes)
	http.HandleFunc("/events", handleGlobalSSE)
	http.HandleFunc("/clients", handleClients)

	log.Printf("Starting SLIM Session Manager on http://localhost%s", httpAddr)
	log.Printf("SLIM server: %s, Local ID: %s", slimEndpoint, localID)
	if err := http.ListenAndServe(httpAddr, nil); err != nil {
		log.Fatalf("Server failed: %v", err)
	}
}

// initSLIM initializes the SLIM connection
func initSLIM(localID, serverAddr, secret string) (*SessionManager, error) {
	app, connID, err := common.CreateAndConnectApp(localID, serverAddr, secret)
	if err != nil {
		return nil, err
	}

	mgr := &SessionManager{
		app:        app,
		connID:     connID,
		localID:    localID,
		sessions:   make(map[string]*SessionInfo),
		messages:   make(map[string][]Message),
		clients:    make(map[string]*KnownClient),
		globalSSE:  make(map[string]chan SSEEvent),
		sessionSSE: make(map[string]map[string]chan SSEEvent),
	}

	// Start listening for incoming sessions
	go mgr.listenForIncomingSessions()

	return mgr, nil
}

// usernameCookieName returns the port-specific cookie name
func usernameCookieName() string {
	return fmt.Sprintf("sessionmgr_user_%d", serverPort)
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

	manager.mu.RLock()
	sessions := make([]*SessionInfo, 0, len(manager.sessions))
	for _, s := range manager.sessions {
		sessions = append(sessions, s)
	}
	manager.mu.RUnlock()

	data := map[string]interface{}{
		"Username": username,
		"Sessions": sessions,
		"LocalID":  manager.localID,
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

// handleSessions handles listing (GET) and creating (POST) sessions
func handleSessions(w http.ResponseWriter, r *http.Request) {
	if r.Method == "POST" {
		handleCreateSession(w, r)
		return
	}

	// GET - return session list partial for HTMX
	manager.mu.RLock()
	sessions := make([]*SessionInfo, 0, len(manager.sessions))
	for _, s := range manager.sessions {
		sessions = append(sessions, s)
	}
	manager.mu.RUnlock()

	templates.ExecuteTemplate(w, "session_list", sessions)
}

// handleCreateSession creates a new session
func handleCreateSession(w http.ResponseWriter, r *http.Request) {
	sessionType := r.FormValue("type")
	destination := strings.TrimSpace(r.FormValue("destination"))
	enableMLS := r.FormValue("enable_mls") == "on"

	if destination == "" {
		http.Error(w, "Destination required", http.StatusBadRequest)
		return
	}

	if sessionType != "p2p" && sessionType != "group" {
		sessionType = "p2p"
	}

	info, err := manager.createSession(sessionType, destination, enableMLS)
	if err != nil {
		log.Printf("Failed to create session: %v", err)
		http.Error(w, "Failed to create session: "+err.Error(), http.StatusInternalServerError)
		return
	}

	log.Printf("Created %s session: %s -> %s", sessionType, info.ID, destination)

	// Redirect to session detail page
	http.Redirect(w, r, "/session/"+info.ID, http.StatusSeeOther)
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

	// Route based on path parts
	if len(parts) == 1 {
		// /session/{id}
		if r.Method == "DELETE" {
			handleDeleteSession(w, r, sessionID)
		} else {
			handleSessionDetail(w, r, sessionID)
		}
		return
	}

	action := parts[1]
	switch action {
	case "invite":
		handleInvite(w, r, sessionID)
	case "remove":
		handleRemove(w, r, sessionID)
	case "send":
		handleSendMessage(w, r, sessionID)
	case "messages":
		handleListMessages(w, r, sessionID)
	case "events":
		handleSessionSSE(w, r, sessionID)
	case "delete":
		handleDeleteSession(w, r, sessionID)
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

	manager.mu.RLock()
	info, exists := manager.sessions[sessionID]
	messages := manager.messages[sessionID]
	manager.mu.RUnlock()

	if !exists {
		http.Error(w, "Session not found", http.StatusNotFound)
		return
	}

	// Get known clients for invite suggestions
	manager.mu.RLock()
	clients := make([]*KnownClient, 0, len(manager.clients))
	for _, c := range manager.clients {
		clients = append(clients, c)
	}
	manager.mu.RUnlock()

	data := map[string]interface{}{
		"Username": username,
		"Session":  info,
		"Messages": messages,
		"Clients":  clients,
		"LocalID":  manager.localID,
	}
	templates.ExecuteTemplate(w, "session_detail", data)
}

// handleDeleteSession deletes a session
func handleDeleteSession(w http.ResponseWriter, r *http.Request, sessionID string) {
	if err := manager.deleteSession(sessionID); err != nil {
		http.Error(w, "Failed to delete session: "+err.Error(), http.StatusInternalServerError)
		return
	}

	log.Printf("Deleted session: %s", sessionID)

	// Check if this is an HTMX request
	if r.Header.Get("HX-Request") == "true" {
		// Return empty response to remove the element
		w.WriteHeader(http.StatusOK)
		return
	}

	http.Redirect(w, r, "/", http.StatusSeeOther)
}

// handleInvite invites a participant to a group session
func handleInvite(w http.ResponseWriter, r *http.Request, sessionID string) {
	if r.Method != "POST" {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	participant := strings.TrimSpace(r.FormValue("participant"))
	if participant == "" {
		http.Error(w, "Participant required", http.StatusBadRequest)
		return
	}

	if err := manager.inviteParticipant(sessionID, participant); err != nil {
		http.Error(w, "Failed to invite: "+err.Error(), http.StatusInternalServerError)
		return
	}

	log.Printf("Invited %s to session %s", participant, sessionID)

	// Return updated participant list
	manager.mu.RLock()
	info := manager.sessions[sessionID]
	manager.mu.RUnlock()

	templates.ExecuteTemplate(w, "participants", info)
}

// handleRemove removes a participant from a group session
func handleRemove(w http.ResponseWriter, r *http.Request, sessionID string) {
	if r.Method != "POST" {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	participant := strings.TrimSpace(r.FormValue("participant"))
	if participant == "" {
		http.Error(w, "Participant required", http.StatusBadRequest)
		return
	}

	if err := manager.removeParticipant(sessionID, participant); err != nil {
		http.Error(w, "Failed to remove: "+err.Error(), http.StatusInternalServerError)
		return
	}

	log.Printf("Removed %s from session %s", participant, sessionID)

	// Return updated participant list
	manager.mu.RLock()
	info := manager.sessions[sessionID]
	manager.mu.RUnlock()

	templates.ExecuteTemplate(w, "participants", info)
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

	if err := manager.sendMessage(sessionID, text); err != nil {
		http.Error(w, "Failed to send: "+err.Error(), http.StatusInternalServerError)
		return
	}

	w.WriteHeader(http.StatusOK)
}

// handleListMessages returns messages for a session
func handleListMessages(w http.ResponseWriter, r *http.Request, sessionID string) {
	manager.mu.RLock()
	messages := manager.messages[sessionID]
	manager.mu.RUnlock()

	templates.ExecuteTemplate(w, "messages", messages)
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

	// Create subscriber channel
	subID := fmt.Sprintf("sub-%d", time.Now().UnixNano())
	events := make(chan SSEEvent, 100)

	manager.sseMu.Lock()
	manager.globalSSE[subID] = events
	manager.sseMu.Unlock()

	defer func() {
		manager.sseMu.Lock()
		delete(manager.globalSSE, subID)
		manager.sseMu.Unlock()
		close(events)
	}()

	// Send initial ping
	fmt.Fprintf(w, "event: ping\ndata: connected\n\n")
	flusher.Flush()

	// Stream events
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

	// Create subscriber channel
	subID := fmt.Sprintf("sub-%d", time.Now().UnixNano())
	events := make(chan SSEEvent, 100)

	manager.sseMu.Lock()
	if manager.sessionSSE[sessionID] == nil {
		manager.sessionSSE[sessionID] = make(map[string]chan SSEEvent)
	}
	manager.sessionSSE[sessionID][subID] = events
	manager.sseMu.Unlock()

	defer func() {
		manager.sseMu.Lock()
		delete(manager.sessionSSE[sessionID], subID)
		manager.sseMu.Unlock()
		close(events)
	}()

	// Send existing messages first
	manager.mu.RLock()
	messages := manager.messages[sessionID]
	manager.mu.RUnlock()

	for _, msg := range messages {
		html := formatMessageHTML(msg)
		fmt.Fprintf(w, "event: message\ndata: %s\n\n", html)
	}
	flusher.Flush()

	// Stream new events
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

// handleClients returns known clients as JSON or datalist options
func handleClients(w http.ResponseWriter, r *http.Request) {
	manager.mu.RLock()
	clients := make([]*KnownClient, 0, len(manager.clients))
	for _, c := range manager.clients {
		clients = append(clients, c)
	}
	manager.mu.RUnlock()

	if r.Header.Get("Accept") == "application/json" {
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(clients)
		return
	}

	// Return as datalist options
	templates.ExecuteTemplate(w, "clients_datalist", clients)
}

// SessionManager methods

func (m *SessionManager) createSession(sessionType, destination string, enableMLS bool) (*SessionInfo, error) {
	destName, err := common.SplitID(destination)
	if err != nil {
		return nil, fmt.Errorf("invalid destination format (use org/namespace/app): %w", err)
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

		config := slim.SessionConfig{
			SessionType: slimType,
			EnableMls:   enableMLS,
			MaxRetries:  &[]uint32{5}[0],    // 5 retries
			IntervalMs:  &[]uint64{5000}[0], // 5 second timeout
			Initiator:   true,
			Metadata:    make(map[string]string),
		}

		session, err = m.app.CreateSession(config, destName)
		if err != nil {
			return nil, fmt.Errorf("failed to create session: %w", err)
		}

		// Give session a moment to establish before returning
		time.Sleep(100 * time.Millisecond)

		sessionID, _ = session.SessionId()
	} else {
		// Demo mode - generate fake session ID
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
	m.messages[info.ID] = []Message{}
	m.mu.Unlock()

	// Start message receiver for this session
	if session != nil {
		go m.receiveMessages(info)
	}

	// Broadcast SSE event
	m.broadcastGlobalEvent(SSEEvent{
		Type: "session_created",
		Data: map[string]interface{}{"id": info.ID, "type": info.Type, "destination": info.Destination},
	})

	return info, nil
}

func (m *SessionManager) deleteSession(sessionID string) error {
	m.mu.Lock()
	info, exists := m.sessions[sessionID]
	if !exists {
		m.mu.Unlock()
		return fmt.Errorf("session not found")
	}
	delete(m.sessions, sessionID)
	delete(m.messages, sessionID)
	m.mu.Unlock()

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

	m.broadcastGlobalEvent(SSEEvent{
		Type: "session_deleted",
		Data: map[string]string{"id": sessionID},
	})

	return nil
}

func (m *SessionManager) inviteParticipant(sessionID, participant string) error {
	m.mu.RLock()
	info, exists := m.sessions[sessionID]
	m.mu.RUnlock()

	if !exists {
		return fmt.Errorf("session not found")
	}
	if info.Type != "group" {
		return fmt.Errorf("can only invite to group sessions")
	}
	if !info.IsInitiator {
		return fmt.Errorf("only session initiator can invite")
	}

	participantName, err := common.SplitID(participant)
	if err != nil {
		return fmt.Errorf("invalid participant format: %w", err)
	}

	if m.app != nil && info.Session != nil {
		// Set route for participant
		if err := m.app.SetRoute(participantName, m.connID); err != nil {
			return fmt.Errorf("failed to set route: %w", err)
		}

		// Send invite (with mutex for thread-safety)
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

	m.broadcastGlobalEvent(SSEEvent{
		Type: "participant_joined",
		Data: map[string]string{"sessionId": sessionID, "participant": participant},
	})

	return nil
}

func (m *SessionManager) removeParticipant(sessionID, participant string) error {
	m.mu.RLock()
	info, exists := m.sessions[sessionID]
	m.mu.RUnlock()

	if !exists {
		return fmt.Errorf("session not found")
	}
	if info.Type != "group" {
		return fmt.Errorf("can only remove from group sessions")
	}
	if !info.IsInitiator {
		return fmt.Errorf("only session initiator can remove")
	}

	participantName, err := common.SplitID(participant)
	if err != nil {
		return fmt.Errorf("invalid participant format: %w", err)
	}

	if m.app != nil && info.Session != nil {
		// Remove participant (with mutex for thread-safety)
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

	m.broadcastGlobalEvent(SSEEvent{
		Type: "participant_removed",
		Data: map[string]string{"sessionId": sessionID, "participant": participant},
	})

	return nil
}

func (m *SessionManager) sendMessage(sessionID, text string) error {
	m.mu.RLock()
	info, exists := m.sessions[sessionID]
	m.mu.RUnlock()

	if !exists {
		return fmt.Errorf("session not found")
	}

	if m.app != nil && info.Session != nil {
		// Publish message (with mutex for thread-safety)
		info.sessionMu.Lock()
		err := info.Session.Publish([]byte(text), nil, nil)
		info.sessionMu.Unlock()
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

	m.mu.Lock()
	m.messages[sessionID] = append(m.messages[sessionID], msg)
	info.MessageCount++
	info.LastActivity = time.Now()
	m.mu.Unlock()

	// Broadcast via SSE
	m.broadcastSessionEvent(sessionID, SSEEvent{
		Type: "message",
		Data: formatMessageHTML(msg),
	})

	return nil
}

func (m *SessionManager) receiveMessages(info *SessionInfo) {
	for {
		select {
		case <-info.stopChan:
			return
		default:
		}

		timeout := uint32(1000)
		// GetMessage (with mutex for thread-safety)
		info.sessionMu.Lock()
		msg, err := info.Session.GetMessage(&timeout)
		info.sessionMu.Unlock()
		if err != nil {
			errStr := err.Error()
			if strings.Contains(errStr, "timeout") || strings.Contains(errStr, "timed out") {
				continue
			}
			// Participant disconnected is informational, not fatal
			if strings.Contains(errStr, "participant disconnected") {
				log.Printf("Session %s: %v", info.ID, err)
				continue
			}
			// Session closed or fatal error
			log.Printf("Session %s GetMessage error: %v", info.ID, err)
			m.mu.Lock()
			info.Status = "closed"
			m.mu.Unlock()

			m.broadcastGlobalEvent(SSEEvent{
				Type: "session_updated",
				Data: map[string]string{"id": info.ID, "status": "closed"},
			})
			return
		}

		// Create message record
		sender := strings.Join(msg.Context.SourceName.Components, "/")
		message := Message{
			ID:        fmt.Sprintf("msg-%d", time.Now().UnixNano()),
			SessionID: info.ID,
			Direction: "received",
			Sender:    sender,
			Text:      string(msg.Payload),
			Metadata:  msg.Context.Metadata,
			Timestamp: time.Now(),
		}

		m.mu.Lock()
		m.messages[info.ID] = append(m.messages[info.ID], message)
		info.MessageCount++
		info.LastActivity = time.Now()
		m.mu.Unlock()

		// Track sender as known client
		m.addKnownClient(sender, "message")

		// Broadcast message via SSE
		m.broadcastSessionEvent(info.ID, SSEEvent{
			Type: "message",
			Data: formatMessageHTML(message),
		})
	}
}

func (m *SessionManager) listenForIncomingSessions() {
	if m.app == nil {
		return
	}

	for {
		timeout := uint32(5000)
		session, err := m.app.ListenForSession(&timeout)
		if err != nil {
			errStr := err.Error()
			if strings.Contains(errStr, "timeout") || strings.Contains(errStr, "timed out") {
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
		m.messages[info.ID] = []Message{}
		m.mu.Unlock()

		log.Printf("Accepted incoming session: %s (%s)", info.ID, info.Destination)

		// Start message receiver
		go m.receiveMessages(info)

		m.broadcastGlobalEvent(SSEEvent{
			Type: "session_created",
			Data: map[string]interface{}{"id": info.ID, "type": info.Type, "destination": info.Destination, "incoming": true},
		})
	}
}

func (m *SessionManager) addKnownClient(name, source string) {
	m.mu.Lock()
	defer m.mu.Unlock()

	m.clients[name] = &KnownClient{
		Name:     name,
		LastSeen: time.Now(),
		Source:   source,
	}
}

func (m *SessionManager) broadcastGlobalEvent(event SSEEvent) {
	m.sseMu.RLock()
	defer m.sseMu.RUnlock()

	for _, ch := range m.globalSSE {
		select {
		case ch <- event:
		default:
			// Channel full, skip
		}
	}
}

func (m *SessionManager) broadcastSessionEvent(sessionID string, event SSEEvent) {
	m.sseMu.RLock()
	defer m.sseMu.RUnlock()

	if subs, ok := m.sessionSSE[sessionID]; ok {
		for _, ch := range subs {
			select {
			case ch <- event:
			default:
				// Channel full, skip
			}
		}
	}
}

// formatMessageHTML formats a message as HTML for SSE
func formatMessageHTML(msg Message) string {
	timestamp := msg.Timestamp.Format("15:04:05")
	class := "message"
	if msg.Direction == "sent" {
		class = "message sent"
	}
	// Escape HTML
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
    <title>Login - SLIM Session Manager</title>
    <style>
        * { box-sizing: border-box; margin: 0; padding: 0; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #1a1a2e; color: #eee; min-height: 100vh; display: flex; align-items: center; justify-content: center; }
        .login-box { background: #16213e; border-radius: 8px; padding: 40px; width: 100%; max-width: 400px; }
        h1 { color: #00d4ff; margin-bottom: 10px; text-align: center; }
        p { color: #888; margin-bottom: 30px; text-align: center; }
        input[type="text"] { width: 100%; padding: 12px; border: 1px solid #333; border-radius: 4px; background: #0f0f23; color: #fff; font-size: 16px; margin-bottom: 15px; }
        input[type="text"]:focus { outline: none; border-color: #00d4ff; }
        button { width: 100%; background: #00d4ff; color: #000; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; font-size: 16px; font-weight: bold; }
        button:hover { background: #00b8e6; }
    </style>
</head>
<body>
    <div class="login-box">
        <h1>SLIM Session Manager</h1>
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
    <title>SLIM Session Manager</title>
    <script src="https://unpkg.com/htmx.org@2.0.4"></script>
    <script src="https://unpkg.com/htmx-ext-sse@2.2.2/sse.js"></script>
    <style>
        * { box-sizing: border-box; margin: 0; padding: 0; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #1a1a2e; color: #eee; min-height: 100vh; }
        .container { max-width: 900px; margin: 0 auto; padding: 20px; }
        .header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px; padding-bottom: 15px; border-bottom: 1px solid #333; }
        h1 { color: #00d4ff; }
        .user-info { display: flex; align-items: center; gap: 15px; }
        .user-info span { color: #00d4ff; }
        .user-info a { color: #888; text-decoration: none; font-size: 14px; }
        .user-info a:hover { color: #fff; }
        .local-id { color: #666; font-size: 12px; }
        h2 { color: #888; font-size: 14px; text-transform: uppercase; margin: 20px 0 10px; }
        .card { background: #16213e; border-radius: 8px; padding: 20px; margin-bottom: 20px; }
        input[type="text"], select { padding: 12px; border: 1px solid #333; border-radius: 4px; background: #0f0f23; color: #fff; font-size: 16px; }
        input[type="text"]:focus, select:focus { outline: none; border-color: #00d4ff; }
        button { background: #00d4ff; color: #000; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; font-size: 16px; font-weight: bold; }
        button:hover { background: #00b8e6; }
        button.danger { background: #e74c3c; color: #fff; }
        button.danger:hover { background: #c0392b; }
        .form-row { display: flex; gap: 10px; flex-wrap: wrap; align-items: center; }
        .form-row input[type="text"] { flex: 1; min-width: 200px; }
        .form-row select { min-width: 120px; }
        label { display: flex; align-items: center; gap: 8px; color: #888; font-size: 14px; }
        label input[type="checkbox"] { width: 18px; height: 18px; }
        .session-list { display: grid; gap: 15px; }
        .session-card { display: flex; justify-content: space-between; align-items: center; padding: 15px; background: #0f0f23; border-radius: 4px; border-left: 4px solid #00d4ff; }
        .session-card.group { border-left-color: #9b59b6; }
        .session-card.closed { opacity: 0.6; border-left-color: #666; }
        .session-info { flex: 1; }
        .session-id { font-weight: bold; color: #00d4ff; font-family: monospace; }
        .session-meta { color: #888; font-size: 14px; margin-top: 5px; }
        .session-meta span { margin-right: 15px; }
        .badge { display: inline-block; padding: 2px 8px; border-radius: 3px; font-size: 12px; text-transform: uppercase; }
        .badge.p2p { background: #00d4ff; color: #000; }
        .badge.group { background: #9b59b6; color: #fff; }
        .badge.active { background: #2ecc71; color: #000; }
        .badge.closed { background: #666; color: #fff; }
        .session-actions { display: flex; gap: 10px; }
        .session-actions a, .session-actions button { padding: 8px 16px; font-size: 14px; text-decoration: none; border-radius: 4px; }
        .session-actions a { background: #4CAF50; color: #fff; }
        .session-actions a:hover { background: #45a049; }
        .empty { color: #666; text-align: center; padding: 40px; }
    </style>
</head>
<body hx-ext="sse" sse-connect="/events">
    <div class="container">
        <div class="header">
            <div>
                <h1>SLIM Session Manager</h1>
                <div class="local-id">Local ID: {{.LocalID}}</div>
            </div>
            <div class="user-info">
                <span>{{.Username}}</span>
                <a href="/logout">Logout</a>
            </div>
        </div>

        <div class="card">
            <h2>Create New Session</h2>
            <form action="/sessions" method="POST" class="form-row">
                <select name="type">
                    <option value="p2p">Point-to-Point</option>
                    <option value="group">Group</option>
                </select>
                <input type="text" name="destination" placeholder="Destination (org/namespace/app)..." required pattern="[^/]+/[^/]+/[^/]+">
                <label>
                    <input type="checkbox" name="enable_mls">
                    Enable MLS
                </label>
                <button type="submit">Create Session</button>
            </form>
        </div>

        <div class="card">
            <h2>Active Sessions</h2>
            <div id="session-list" hx-get="/sessions" hx-trigger="sse:session_created, sse:session_deleted, sse:session_updated, every 10s">
                {{template "session_list" .Sessions}}
            </div>
        </div>
    </div>
</body>
</html>
{{end}}

{{define "session_list"}}
{{if .}}
<div class="session-list">
    {{range .}}
    <div class="session-card {{.Type}} {{.Status}}" id="session-{{.ID}}">
        <div class="session-info">
            <div>
                <span class="badge {{.Type}}">{{.Type}}</span>
                <span class="badge {{.Status}}">{{.Status}}</span>
                {{if .EnableMLS}}<span class="badge" style="background:#f39c12;color:#000">MLS</span>{{end}}
            </div>
            <div class="session-id">{{.ID}}</div>
            <div class="session-meta">
                <span>To: {{.Destination}}</span>
                {{if eq .Type "group"}}<span>Participants: {{len .Participants}}</span>{{end}}
                <span>Messages: {{.MessageCount}}</span>
            </div>
        </div>
        <div class="session-actions">
            <a href="/session/{{.ID}}">Open</a>
            <button class="danger" hx-post="/session/{{.ID}}/delete" hx-target="#session-{{.ID}}" hx-swap="outerHTML" hx-confirm="Delete this session?">Delete</button>
        </div>
    </div>
    {{end}}
</div>
{{else}}
<p class="empty">No active sessions. Create one above!</p>
{{end}}
{{end}}

{{define "session_detail"}}
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Session {{.Session.ID}} - SLIM Session Manager</title>
    <script src="https://unpkg.com/htmx.org@2.0.4"></script>
    <script src="https://unpkg.com/htmx-ext-sse@2.2.2/sse.js"></script>
    <style>
        * { box-sizing: border-box; margin: 0; padding: 0; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #1a1a2e; color: #eee; min-height: 100vh; display: flex; flex-direction: column; }
        .header { background: #16213e; padding: 15px 20px; display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid #333; }
        .header h1 { font-size: 18px; color: #00d4ff; }
        .header-left { display: flex; align-items: center; gap: 20px; }
        .header a { color: #888; text-decoration: none; }
        .header a:hover { color: #fff; }
        .badge { display: inline-block; padding: 2px 8px; border-radius: 3px; font-size: 12px; text-transform: uppercase; margin-left: 10px; }
        .badge.p2p { background: #00d4ff; color: #000; }
        .badge.group { background: #9b59b6; color: #fff; }
        .badge.active { background: #2ecc71; color: #000; }
        .main { display: flex; flex: 1; overflow: hidden; }
        .sidebar { width: 280px; background: #16213e; border-right: 1px solid #333; padding: 15px; overflow-y: auto; }
        .sidebar h3 { color: #888; font-size: 12px; text-transform: uppercase; margin-bottom: 10px; }
        .info-item { margin-bottom: 15px; }
        .info-label { color: #666; font-size: 12px; }
        .info-value { color: #fff; font-family: monospace; word-break: break-all; }
        .participant-list { list-style: none; }
        .participant-item { display: flex; justify-content: space-between; align-items: center; padding: 8px; background: #0f0f23; border-radius: 4px; margin-bottom: 5px; font-size: 14px; }
        .participant-item button { padding: 4px 8px; font-size: 12px; background: #e74c3c; color: #fff; border: none; border-radius: 3px; cursor: pointer; }
        .invite-form { margin-top: 15px; }
        .invite-form input { width: 100%; padding: 8px; border: 1px solid #333; border-radius: 4px; background: #0f0f23; color: #fff; font-size: 14px; margin-bottom: 8px; }
        .invite-form button { width: 100%; padding: 8px; background: #9b59b6; color: #fff; border: none; border-radius: 4px; cursor: pointer; font-size: 14px; }
        .chat-area { flex: 1; display: flex; flex-direction: column; }
        .messages { flex: 1; overflow-y: auto; padding: 20px; }
        .message { padding: 8px 0; border-bottom: 1px solid #222; }
        .message .time { color: #666; }
        .message .sender { color: #00d4ff; font-weight: bold; }
        .message.sent .sender { color: #2ecc71; }
        .input-area { background: #16213e; padding: 15px 20px; border-top: 1px solid #333; }
        .input-row { display: flex; gap: 10px; }
        input[type="text"] { flex: 1; padding: 12px; border: 1px solid #333; border-radius: 4px; background: #0f0f23; color: #fff; font-size: 16px; }
        input[type="text"]:focus { outline: none; border-color: #00d4ff; }
        button { background: #00d4ff; color: #000; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; font-size: 16px; font-weight: bold; }
        button:hover { background: #00b8e6; }
        .empty-messages { color: #666; text-align: center; padding: 40px; }
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
        <button class="danger" style="background:#e74c3c;color:#fff" hx-post="/session/{{.Session.ID}}/delete" hx-confirm="Delete this session?" onclick="if(event.target.closest('[hx-post]').getAttribute('hx-confirm') && !confirm('Delete this session?')) return false; window.location='/'">Delete</button>
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
                <div class="info-label">Created</div>
                <div class="info-value">{{formatDate .Session.CreatedAt}}</div>
            </div>
            <div class="info-item">
                <div class="info-label">Messages</div>
                <div class="info-value">{{.Session.MessageCount}}</div>
            </div>
            {{if .Session.EnableMLS}}
            <div class="info-item">
                <div class="info-label">Encryption</div>
                <div class="info-value">MLS Enabled</div>
            </div>
            {{end}}

            {{if eq .Session.Type "group"}}
            <h3 style="margin-top:20px">Participants ({{len .Session.Participants}})</h3>
            <div id="participants">
                {{template "participants" .Session}}
            </div>

            {{if .Session.IsInitiator}}
            <div class="invite-form">
                <form hx-post="/session/{{.Session.ID}}/invite" hx-target="#participants" hx-swap="innerHTML" hx-on::after-request="this.reset()">
                    <input type="text" name="participant" list="known-clients" placeholder="org/namespace/app" required pattern="[^/]+/[^/]+/[^/]+">
                    <datalist id="known-clients">
                        {{range .Clients}}
                        <option value="{{.Name}}">{{.Name}}</option>
                        {{end}}
                    </datalist>
                    <button type="submit">Invite Participant</button>
                </form>
            </div>
            {{end}}
            {{end}}
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
        // Auto-scroll to bottom on new messages
        const messages = document.getElementById('messages');
        const observer = new MutationObserver(() => {
            messages.scrollTop = messages.scrollHeight;
        });
        observer.observe(messages, { childList: true });
    </script>
</body>
</html>
{{end}}

{{define "participants"}}
{{if .Participants}}
<ul class="participant-list">
    {{range .Participants}}
    <li class="participant-item">
        <span>{{.}}</span>
        {{if $.IsInitiator}}
        <form hx-post="/session/{{$.ID}}/remove" hx-target="#participants" hx-swap="innerHTML" style="display:inline">
            <input type="hidden" name="participant" value="{{.}}">
            <button type="submit">Remove</button>
        </form>
        {{end}}
    </li>
    {{end}}
</ul>
{{else}}
<p style="color:#666;font-size:14px">No participants yet</p>
{{end}}
{{end}}

{{define "messages"}}
{{range .}}
<div class="message {{.Direction}}">
    <span class="time">[{{formatTime .Timestamp}}]</span>
    <span class="sender">{{.Sender}}:</span>
    {{.Text}}
</div>
{{else}}
<p class="empty-messages">No messages yet. Start the conversation!</p>
{{end}}
{{end}}

{{define "clients_datalist"}}
{{range .}}
<option value="{{.Name}}">{{.Name}} ({{.Source}})</option>
{{end}}
{{end}}
`
