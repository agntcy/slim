// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Package main implements a web-based chat application using SLIM and HTMX.
//
// Usage:
//
//	# First start the SLIM server
//	task example:server
//
//	# Then start the web chat
//	task example:webchat
//
//	# Open http://localhost:8080 in your browser
package main

import (
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
	defaultPort     = 8080
	defaultSlimAddr = "http://localhost:46357"
	defaultSecret   = "demo-shared-secret-min-32-chars!!"
)

// Control channel for room discovery
const controlChannelID = "webchat/control/rooms"

var (
	slimEndpoint string
	slimSecret   string
	serverID     string // Unique ID for this server instance
	serverPort   int    // HTTP server port (used for cookie scoping)

	// Global SLIM connection for control channel
	globalApp     *slim.BindingsAdapter
	globalConnID  uint64
	controlSession *slim.BindingsSessionContext
)

// Message represents a chat message
type Message struct {
	Username  string
	Text      string
	Timestamp time.Time
}

// User represents a connected user in a room
type User struct {
	Username string
	Events   chan Message
}

// Room represents a chat room
type Room struct {
	Name     string
	App      *slim.BindingsAdapter
	Session  *slim.BindingsSessionContext
	ConnID   uint64
	Users    map[string]*User
	Messages []Message
	mu       sync.RWMutex
	stopChan chan struct{}
}

// RoomManager manages all chat rooms
type RoomManager struct {
	rooms map[string]*Room
	mu    sync.RWMutex
}

var (
	manager   *RoomManager
	templates *template.Template
)

func main() {
	// Parse command-line flags
	port := flag.Int("port", defaultPort, "HTTP server port")
	flag.StringVar(&slimEndpoint, "slim", defaultSlimAddr, "SLIM server endpoint")
	flag.StringVar(&slimSecret, "secret", defaultSecret, "SLIM shared secret")
	flag.Parse()

	httpAddr := fmt.Sprintf(":%d", *port)
	serverPort = *port

	// Generate unique server ID
	serverID = fmt.Sprintf("server-%d-%d", *port, time.Now().UnixNano())

	// Initialize crypto provider
	slim.InitializeCryptoProvider()

	// Initialize room manager
	manager = &RoomManager{
		rooms: make(map[string]*Room),
	}

	// Connect to SLIM and start control channel
	if err := initSLIMConnection(*port); err != nil {
		log.Printf("Warning: SLIM connection failed, running in standalone mode: %v", err)
	} else {
		// Start listening for room announcements
		go listenForRoomAnnouncements()
	}

	// Parse templates
	var err error
	templates, err = template.New("").Parse(templatesHTML)
	if err != nil {
		log.Fatalf("Failed to parse templates: %v", err)
	}

	// Set up routes
	http.HandleFunc("/", handleHome)
	http.HandleFunc("/login", handleLogin)
	http.HandleFunc("/logout", handleLogout)
	http.HandleFunc("/rooms", handleRooms)
	http.HandleFunc("/room/", handleRoom)
	http.HandleFunc("/join/", handleJoin)
	http.HandleFunc("/send/", handleSend)
	http.HandleFunc("/events/", handleEvents)
	http.HandleFunc("/leave/", handleLeave)

	log.Printf("Starting web chat server on http://localhost%s", httpAddr)
	log.Printf("SLIM server: %s (serverID: %s)", slimEndpoint, serverID)
	if err := http.ListenAndServe(httpAddr, nil); err != nil {
		log.Fatalf("Server failed: %v", err)
	}
}

// usernameCookieName returns the port-specific cookie name for username
// This prevents cookie conflicts when running multiple servers on localhost
func usernameCookieName() string {
	return fmt.Sprintf("username_%d", serverPort)
}

// getUsername returns the global username from cookie, or empty string if not set
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

	// Check if user has set their username
	username := getUsername(r)
	if username == "" {
		// Show login page
		templates.ExecuteTemplate(w, "login", nil)
		return
	}

	manager.mu.RLock()
	rooms := make([]map[string]interface{}, 0, len(manager.rooms))
	for name, room := range manager.rooms {
		room.mu.RLock()
		rooms = append(rooms, map[string]interface{}{
			"Name":      name,
			"UserCount": len(room.Users),
		})
		room.mu.RUnlock()
	}
	manager.mu.RUnlock()

	data := map[string]interface{}{
		"Rooms":    rooms,
		"Username": username,
	}
	templates.ExecuteTemplate(w, "home", data)
}

// handleLogin sets the global username
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

	// Set global username cookie (port-specific to avoid conflicts on localhost)
	http.SetCookie(w, &http.Cookie{
		Name:   usernameCookieName(),
		Value:  username,
		Path:   "/",
		MaxAge: 86400 * 7, // 7 days
	})

	log.Printf("User logged in: %s (cookie: %s)", username, usernameCookieName())
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

// handleRooms creates a new room (POST) or lists rooms (GET)
func handleRooms(w http.ResponseWriter, r *http.Request) {
	if r.Method == "POST" {
		roomName := strings.TrimSpace(r.FormValue("room"))
		if roomName == "" {
			http.Error(w, "Room name required", http.StatusBadRequest)
			return
		}

		// Check if room already exists
		manager.mu.RLock()
		_, exists := manager.rooms[roomName]
		manager.mu.RUnlock()

		if exists {
			// Room exists, redirect to it
			http.Redirect(w, r, "/room/"+roomName, http.StatusSeeOther)
			return
		}

		// Create new room
		room, err := createRoom(roomName)
		if err != nil {
			log.Printf("Failed to create room %s: %v", roomName, err)
			http.Error(w, "Failed to create room: "+err.Error(), http.StatusInternalServerError)
			return
		}

		manager.mu.Lock()
		manager.rooms[roomName] = room
		manager.mu.Unlock()

		log.Printf("Created room: %s", roomName)
		http.Redirect(w, r, "/room/"+roomName, http.StatusSeeOther)
		return
	}

	// GET - return room list partial
	manager.mu.RLock()
	rooms := make([]map[string]interface{}, 0, len(manager.rooms))
	for name, room := range manager.rooms {
		room.mu.RLock()
		rooms = append(rooms, map[string]interface{}{
			"Name":      name,
			"UserCount": len(room.Users),
		})
		room.mu.RUnlock()
	}
	manager.mu.RUnlock()

	templates.ExecuteTemplate(w, "room_list", rooms)
}

// handleRoom serves the room page
func handleRoom(w http.ResponseWriter, r *http.Request) {
	roomName := strings.TrimPrefix(r.URL.Path, "/room/")
	if roomName == "" {
		http.Redirect(w, r, "/", http.StatusSeeOther)
		return
	}

	// Check for global username - redirect to login if not set
	username := getUsername(r)
	if username == "" {
		http.Redirect(w, r, "/", http.StatusSeeOther)
		return
	}

	manager.mu.RLock()
	room, exists := manager.rooms[roomName]
	manager.mu.RUnlock()

	if !exists {
		http.Error(w, "Room not found", http.StatusNotFound)
		return
	}

	// Auto-join the room if not already joined
	room.mu.Lock()
	if room.Users[username] == nil {
		room.Users[username] = &User{
			Username: username,
			Events:   make(chan Message, 100),
		}
		room.mu.Unlock()

		// Broadcast join message
		joinMsg := Message{
			Username:  "system",
			Text:      username + " joined the room",
			Timestamp: time.Now(),
		}
		broadcastMessage(room, joinMsg)
		log.Printf("User %s joined room %s", username, roomName)
	} else {
		room.mu.Unlock()
	}

	room.mu.RLock()
	data := map[string]interface{}{
		"RoomName": roomName,
		"Username": username,
		"Messages": room.Messages,
	}
	room.mu.RUnlock()

	templates.ExecuteTemplate(w, "room", data)
}

// handleJoin handles user joining a room (legacy endpoint, redirects to room)
func handleJoin(w http.ResponseWriter, r *http.Request) {
	roomName := strings.TrimPrefix(r.URL.Path, "/join/")
	if roomName == "" {
		http.Redirect(w, r, "/", http.StatusSeeOther)
		return
	}
	// Auto-join now happens in handleRoom
	http.Redirect(w, r, "/room/"+roomName, http.StatusSeeOther)
}

// handleSend handles sending messages
func handleSend(w http.ResponseWriter, r *http.Request) {
	if r.Method != "POST" {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	roomName := strings.TrimPrefix(r.URL.Path, "/send/")
	message := strings.TrimSpace(r.FormValue("message"))

	username := getUsername(r)
	if username == "" {
		http.Error(w, "Not logged in", http.StatusUnauthorized)
		return
	}

	if message == "" {
		w.WriteHeader(http.StatusOK)
		return
	}

	manager.mu.RLock()
	room, exists := manager.rooms[roomName]
	manager.mu.RUnlock()

	if !exists {
		http.Error(w, "Room not found", http.StatusNotFound)
		return
	}

	log.Printf("[SEND] handleSend: username='%s' room='%s' message='%s'", username, roomName, message)

	msg := Message{
		Username:  username,
		Text:      message,
		Timestamp: time.Now(),
	}

	// Store and broadcast to local users
	broadcastMessage(room, msg)

	// Broadcast to other servers via SLIM control channel
	broadcastChatMessage(roomName, username, message)

	w.WriteHeader(http.StatusOK)
}

// handleEvents handles SSE connections
func handleEvents(w http.ResponseWriter, r *http.Request) {
	roomName := strings.TrimPrefix(r.URL.Path, "/events/")

	username := getUsername(r)
	if username == "" {
		http.Error(w, "Not logged in", http.StatusUnauthorized)
		return
	}

	manager.mu.RLock()
	room, exists := manager.rooms[roomName]
	manager.mu.RUnlock()

	if !exists {
		http.Error(w, "Room not found", http.StatusNotFound)
		return
	}

	room.mu.RLock()
	user, userExists := room.Users[username]
	room.mu.RUnlock()

	if !userExists {
		http.Error(w, "User not in room", http.StatusUnauthorized)
		return
	}

	// Set SSE headers
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	w.Header().Set("Connection", "keep-alive")
	w.Header().Set("X-Accel-Buffering", "no")

	flusher, ok := w.(http.Flusher)
	if !ok {
		http.Error(w, "SSE not supported", http.StatusInternalServerError)
		return
	}

	// Send existing messages first
	room.mu.RLock()
	for _, msg := range room.Messages {
		html := formatMessageHTML(msg)
		fmt.Fprintf(w, "event: message\ndata: %s\n\n", html)
	}
	room.mu.RUnlock()
	flusher.Flush()

	// Stream new messages
	for {
		select {
		case msg := <-user.Events:
			html := formatMessageHTML(msg)
			fmt.Fprintf(w, "event: message\ndata: %s\n\n", html)
			flusher.Flush()
		case <-r.Context().Done():
			return
		}
	}
}

// handleLeave handles user leaving a room
func handleLeave(w http.ResponseWriter, r *http.Request) {
	roomName := strings.TrimPrefix(r.URL.Path, "/leave/")

	username := getUsername(r)
	if username != "" {
		manager.mu.RLock()
		room, exists := manager.rooms[roomName]
		manager.mu.RUnlock()

		if exists {
			room.mu.Lock()
			delete(room.Users, username)
			room.mu.Unlock()

			// Broadcast leave message
			leaveMsg := Message{
				Username:  "system",
				Text:      username + " left the room",
				Timestamp: time.Now(),
			}
			broadcastMessage(room, leaveMsg)
		}
	}

	http.Redirect(w, r, "/", http.StatusSeeOther)
}

// Dynamic server discovery - no hardcoded ports needed
var (
	knownServers   = make(map[string]bool) // serverID -> online status
	knownServersMu sync.RWMutex
)

// initSLIMConnection connects to SLIM server and joins the control session
// The control session is created by the SLIM server with --webchat flag
func initSLIMConnection(port int) error {
	localID := fmt.Sprintf("webchat/server/%d", port)

	app, connID, err := common.CreateAndConnectApp(localID, slimEndpoint, slimSecret)
	if err != nil {
		return fmt.Errorf("failed to connect: %w", err)
	}

	globalApp = app
	globalConnID = connID

	// Set route for control channel
	controlName, err := common.SplitID(controlChannelID)
	if err != nil {
		return fmt.Errorf("invalid control channel: %w", err)
	}

	if err := app.SetRoute(controlName, connID); err != nil {
		return fmt.Errorf("failed to set control route: %w", err)
	}

	// Wait for invitation from the coordinator (SLIM server with --webchat)
	log.Printf("Waiting for control session invitation from SLIM server...")
	log.Printf("(Make sure SLIM server is running with --webchat flag)")

	timeout := uint32(30000) // 30 seconds
	session, err := app.ListenForSession(&timeout)
	if err != nil {
		return fmt.Errorf("failed to join control session (is SLIM server running with --webchat?): %w", err)
	}

	controlSession = session
	log.Printf("Joined control session")

	// Announce ourselves
	go announceServerOnline()

	return nil
}

// reconnectToControlSession attempts to rejoin the control session
func reconnectToControlSession() bool {
	for attempt := 1; attempt <= 5; attempt++ {
		log.Printf("Reconnect attempt %d/5...", attempt)
		timeout := uint32(5000)
		session, err := globalApp.ListenForSession(&timeout)
		if err == nil {
			controlSession = session
			log.Printf("Successfully reconnected to control session")
			go announceServerOnline()
			return true
		}
		time.Sleep(2 * time.Second)
	}
	return false
}

// announceServerOnline broadcasts that this server is online
func announceServerOnline() {
	// Wait a moment for session to be ready
	time.Sleep(500 * time.Millisecond)

	metadata := map[string]string{
		"type":   "SERVER_ONLINE",
		"server": serverID,
	}

	if err := controlSession.Publish([]byte(""), nil, &metadata); err != nil {
		log.Printf("Failed to announce server online: %v", err)
	} else {
		log.Printf("Announced server online: %s", serverID)
	}

	// Periodically re-announce (heartbeat) every 30 seconds
	ticker := time.NewTicker(30 * time.Second)
	defer ticker.Stop()

	for range ticker.C {
		if controlSession == nil {
			return
		}
		controlSession.Publish([]byte(""), nil, &metadata)
	}
}

// handleServerOnline processes a server coming online
func handleServerOnline(remoteServerID string) {
	knownServersMu.Lock()
	isNew := !knownServers[remoteServerID]
	knownServers[remoteServerID] = true
	knownServersMu.Unlock()

	if isNew {
		log.Printf("Discovered peer server: %s", remoteServerID)
		// Send our room list to the new server
		sendRoomList()
	}
}

// listenForRoomAnnouncements listens for room creation events from other servers
func listenForRoomAnnouncements() {
	if controlSession == nil {
		return
	}

	// Wait longer for session to stabilize before requesting room list
	time.Sleep(2 * time.Second)
	requestRoomList()
	log.Printf("[DEBUG] Started listening for room announcements")

	for {
		timeout := uint32(1000)
		msg, err := controlSession.GetMessage(&timeout)
		if err != nil {
			errStr := err.Error()
			if strings.Contains(errStr, "timeout") {
				continue
			}
			// Handle session closed - try to reconnect
			if strings.Contains(errStr, "closed") || strings.Contains(errStr, "disconnected") {
				log.Printf("Control session closed, attempting to reconnect...")
				if reconnectToControlSession() {
					log.Printf("Reconnected to control session")
					requestRoomList()
					continue
				}
				log.Printf("Failed to reconnect, stopping room announcements listener")
				return
			}
			log.Printf("Control channel error: %v", err)
			return
		}

		// Parse message
		msgType := msg.Context.Metadata["type"]
		roomName := msg.Context.Metadata["room"]
		fromServer := msg.Context.Metadata["server"]

		// Ignore our own messages
		if fromServer == serverID {
			continue
		}

		switch msgType {
		case "SERVER_ONLINE":
			handleServerOnline(fromServer)
		case "ROOM_CREATED":
			log.Printf("Received room announcement: %s from %s", roomName, fromServer)
			handleRemoteRoomCreated(roomName)
		case "CHAT_MESSAGE":
			handleRemoteChatMessage(roomName, msg)
		case "ROOM_LIST_REQUEST":
			log.Printf("Received room list request from %s", fromServer)
			sendRoomList()
		case "ROOM_LIST":
			log.Printf("Received room list from %s: %s", fromServer, string(msg.Payload))
			handleRoomList(string(msg.Payload))
		}
	}
}

// requestRoomList asks other servers for their room lists
func requestRoomList() {
	if controlSession == nil {
		return
	}

	metadata := map[string]string{
		"type":   "ROOM_LIST_REQUEST",
		"server": serverID,
	}

	controlSession.Publish([]byte(""), nil, &metadata)
	log.Printf("Requested room list from other servers")
}

// sendRoomList broadcasts current room list to other servers
func sendRoomList() {
	if controlSession == nil {
		log.Printf("[DEBUG] sendRoomList: no control session")
		return
	}

	manager.mu.RLock()
	roomNames := make([]string, 0, len(manager.rooms))
	for name := range manager.rooms {
		roomNames = append(roomNames, name)
	}
	manager.mu.RUnlock()

	log.Printf("[DEBUG] sendRoomList: %d rooms to send", len(roomNames))

	if len(roomNames) == 0 {
		// Send empty list so other servers know we responded
		metadata := map[string]string{
			"type":   "ROOM_LIST",
			"server": serverID,
		}
		controlSession.Publish([]byte(""), nil, &metadata)
		return
	}

	roomList := strings.Join(roomNames, ",")
	metadata := map[string]string{
		"type":   "ROOM_LIST",
		"server": serverID,
	}

	controlSession.Publish([]byte(roomList), nil, &metadata)
	log.Printf("Sent room list: %s", roomList)
}

// handleRoomList processes a room list from another server
func handleRoomList(roomList string) {
	log.Printf("[DEBUG] handleRoomList received: '%s'", roomList)
	if roomList == "" {
		log.Printf("[DEBUG] handleRoomList: empty list, nothing to do")
		return
	}

	rooms := strings.Split(roomList, ",")
	for _, roomName := range rooms {
		roomName = strings.TrimSpace(roomName)
		if roomName != "" {
			log.Printf("[DEBUG] handleRoomList: creating room '%s'", roomName)
			handleRemoteRoomCreated(roomName)
		}
	}
}

// announceRoom broadcasts a room creation to other servers
func announceRoom(roomName string) {
	if controlSession == nil {
		log.Printf("[DEBUG] announceRoom: no control session, skipping")
		return
	}

	metadata := map[string]string{
		"type":   "ROOM_CREATED",
		"room":   roomName,
		"server": serverID,
	}

	log.Printf("[DEBUG] Announcing room creation: %s", roomName)
	if err := controlSession.Publish([]byte(roomName), nil, &metadata); err != nil {
		log.Printf("Failed to announce room %s: %v", roomName, err)
	} else {
		log.Printf("Announced room: %s", roomName)
	}
}

// handleRemoteRoomCreated creates a local room when another server announces one
func handleRemoteRoomCreated(roomName string) {
	manager.mu.Lock()
	defer manager.mu.Unlock()

	// Check if room already exists
	if _, exists := manager.rooms[roomName]; exists {
		return
	}

	// Create local room (without SLIM session - we'll receive messages via control channel)
	room := &Room{
		Name:     roomName,
		Users:    make(map[string]*User),
		Messages: make([]Message, 0),
		stopChan: make(chan struct{}),
	}

	manager.rooms[roomName] = room
	log.Printf("Created local room from remote: %s", roomName)
}

// handleRemoteChatMessage handles a chat message from another server
func handleRemoteChatMessage(roomName string, msg slim.ReceivedMessage) {
	manager.mu.RLock()
	room, exists := manager.rooms[roomName]
	manager.mu.RUnlock()

	if !exists {
		return
	}

	username := msg.Context.Metadata["username"]
	text := string(msg.Payload)

	// Debug: log all metadata received
	log.Printf("[RECV] Remote message metadata: %+v", msg.Context.Metadata)
	log.Printf("[RECV] Remote message from '%s' in room '%s': %s", username, roomName, text)
	log.Printf("[RECV] Source name: %+v", msg.Context.SourceName)

	chatMsg := Message{
		Username:  username,
		Text:      text,
		Timestamp: time.Now(),
	}

	// Broadcast to local users only (don't re-publish to SLIM)
	room.mu.Lock()
	room.Messages = append(room.Messages, chatMsg)
	if len(room.Messages) > 100 {
		room.Messages = room.Messages[len(room.Messages)-100:]
	}
	room.mu.Unlock()

	room.mu.RLock()
	for _, user := range room.Users {
		select {
		case user.Events <- chatMsg:
		default:
		}
	}
	room.mu.RUnlock()
}

// broadcastChatMessage sends a chat message to other servers via SLIM
func broadcastChatMessage(roomName, username, text string) {
	if controlSession == nil {
		return
	}

	metadata := map[string]string{
		"type":     "CHAT_MESSAGE",
		"room":     roomName,
		"server":   serverID,
		"username": username,
	}

	log.Printf("[SEND] Broadcasting message from '%s' in room '%s': %s", username, roomName, text)

	if err := controlSession.Publish([]byte(text), nil, &metadata); err != nil {
		log.Printf("Failed to broadcast message: %v", err)
	}
}

// createRoom creates a new room and announces it to other servers
func createRoom(name string) (*Room, error) {
	room := &Room{
		Name:     name,
		Users:    make(map[string]*User),
		Messages: make([]Message, 0),
		stopChan: make(chan struct{}),
	}

	// Announce room to other servers
	go announceRoom(name)

	return room, nil
}

// broadcastMessage sends a message to all users in a room
func broadcastMessage(room *Room, msg Message) {
	room.mu.Lock()
	room.Messages = append(room.Messages, msg)
	// Keep last 100 messages
	if len(room.Messages) > 100 {
		room.Messages = room.Messages[len(room.Messages)-100:]
	}
	room.mu.Unlock()

	room.mu.RLock()
	for _, user := range room.Users {
		select {
		case user.Events <- msg:
		default:
			// Channel full, skip
		}
	}
	room.mu.RUnlock()
}

// formatMessageHTML formats a message as HTML for SSE
func formatMessageHTML(msg Message) string {
	timestamp := msg.Timestamp.Format("15:04:05")
	class := "message"
	if msg.Username == "system" {
		class = "message system"
	}
	// Escape HTML
	text := template.HTMLEscapeString(msg.Text)
	username := template.HTMLEscapeString(msg.Username)
	return fmt.Sprintf(`<div class="%s"><span class="time">[%s]</span> <span class="user">%s:</span> %s</div>`,
		class, timestamp, username, text)
}

// ptr is a helper to create pointers to values
func ptr[T any](v T) *T {
	return &v
}

// Embedded HTML templates
const templatesHTML = `
{{define "login"}}
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Login - SLIM Web Chat</title>
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
        <h1>SLIM Web Chat</h1>
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
    <title>SLIM Web Chat</title>
    <script src="https://unpkg.com/htmx.org@2.0.4"></script>
    <style>
        * { box-sizing: border-box; margin: 0; padding: 0; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #1a1a2e; color: #eee; min-height: 100vh; }
        .container { max-width: 600px; margin: 0 auto; padding: 20px; }
        .header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px; }
        h1 { color: #00d4ff; }
        .user-info { display: flex; align-items: center; gap: 15px; }
        .user-info span { color: #00d4ff; }
        .user-info a { color: #888; text-decoration: none; font-size: 14px; }
        .user-info a:hover { color: #fff; }
        h2 { color: #888; font-size: 14px; text-transform: uppercase; margin: 20px 0 10px; }
        .card { background: #16213e; border-radius: 8px; padding: 20px; margin-bottom: 20px; }
        input[type="text"] { width: 100%; padding: 12px; border: 1px solid #333; border-radius: 4px; background: #0f0f23; color: #fff; font-size: 16px; }
        input[type="text"]:focus { outline: none; border-color: #00d4ff; }
        button { background: #00d4ff; color: #000; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; font-size: 16px; font-weight: bold; }
        button:hover { background: #00b8e6; }
        .room-list { list-style: none; }
        .room-item { display: flex; justify-content: space-between; align-items: center; padding: 15px; background: #0f0f23; border-radius: 4px; margin-bottom: 10px; }
        .room-name { font-weight: bold; color: #00d4ff; }
        .room-users { color: #888; font-size: 14px; }
        .join-btn { background: #4CAF50; padding: 8px 16px; font-size: 14px; text-decoration: none; color: #fff; border-radius: 4px; }
        .join-btn:hover { background: #45a049; }
        .empty { color: #666; text-align: center; padding: 40px; }
        .form-row { display: flex; gap: 10px; }
        .form-row input { flex: 1; }
    </style>
</head>
<body>
    <div class="container">
        <div class="header">
            <h1>SLIM Web Chat</h1>
            <div class="user-info">
                <span>{{.Username}}</span>
                <a href="/logout">Logout</a>
            </div>
        </div>

        <div class="card">
            <h2>Create New Room</h2>
            <form action="/rooms" method="POST" class="form-row">
                <input type="text" name="room" placeholder="Room name..." required>
                <button type="submit">Create</button>
            </form>
        </div>

        <div class="card">
            <h2>Active Rooms</h2>
            <div id="room-list" hx-get="/rooms" hx-trigger="every 5s">
                {{template "room_list" .Rooms}}
            </div>
        </div>
    </div>
</body>
</html>
{{end}}

{{define "room_list"}}
{{if .}}
<ul class="room-list">
    {{range .}}
    <li class="room-item">
        <div>
            <span class="room-name">{{.Name}}</span>
            <span class="room-users">({{.UserCount}} users)</span>
        </div>
        <a href="/room/{{.Name}}" class="join-btn">Join</a>
    </li>
    {{end}}
</ul>
{{else}}
<p class="empty">No active rooms. Create one above!</p>
{{end}}
{{end}}

{{define "room"}}
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{{.RoomName}} - SLIM Web Chat</title>
    <script src="https://unpkg.com/htmx.org@2.0.4"></script>
    <script src="https://unpkg.com/htmx-ext-sse@2.2.2/sse.js"></script>
    <style>
        * { box-sizing: border-box; margin: 0; padding: 0; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #1a1a2e; color: #eee; height: 100vh; display: flex; flex-direction: column; }
        .header { background: #16213e; padding: 15px 20px; display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid #333; }
        .header h1 { font-size: 18px; color: #00d4ff; }
        .header .user-info { display: flex; align-items: center; gap: 15px; }
        .header .username { color: #00d4ff; }
        .header a { color: #888; text-decoration: none; }
        .header a:hover { color: #fff; }
        .messages { flex: 1; overflow-y: auto; padding: 20px; }
        .message { padding: 8px 0; border-bottom: 1px solid #222; }
        .message .time { color: #666; }
        .message .user { color: #00d4ff; font-weight: bold; }
        .message.system { color: #888; font-style: italic; }
        .message.system .user { color: #888; }
        .input-area { background: #16213e; padding: 15px 20px; border-top: 1px solid #333; }
        .input-row { display: flex; gap: 10px; }
        input[type="text"] { flex: 1; padding: 12px; border: 1px solid #333; border-radius: 4px; background: #0f0f23; color: #fff; font-size: 16px; }
        input[type="text"]:focus { outline: none; border-color: #00d4ff; }
        button { background: #00d4ff; color: #000; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; font-size: 16px; font-weight: bold; }
        button:hover { background: #00b8e6; }
    </style>
</head>
<body>
    <div class="header">
        <h1>Room: {{.RoomName}}</h1>
        <div class="user-info">
            <span class="username">{{.Username}}</span>
            <a href="/leave/{{.RoomName}}">Leave Room</a>
        </div>
    </div>

    <div class="messages" id="messages" hx-ext="sse" sse-connect="/events/{{.RoomName}}" sse-swap="message" hx-swap="beforeend">
    </div>

    <div class="input-area">
        <form hx-post="/send/{{.RoomName}}" hx-swap="none" hx-on::after-request="this.reset()">
            <div class="input-row">
                <input type="text" name="message" placeholder="Type a message..." autocomplete="off" autofocus>
                <button type="submit">Send</button>
            </div>
        </form>
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
`
