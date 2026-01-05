// Package handler provides HTTP handlers for sessionmgr.
package handler

import (
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"strings"
	"time"

	"github.com/agntcy/slim/bindings/go/examples/internal/httputil"
	"github.com/agntcy/slim/bindings/go/examples/internal/message"
	tmpl "github.com/agntcy/slim/bindings/go/examples/internal/template"
	"github.com/agntcy/slim/bindings/go/examples/sessionmgr/internal/manager"
)

// Handler provides HTTP request handlers for sessionmgr.
type Handler struct {
	manager    *manager.Manager
	auth       *httputil.AuthHandler
	renderer   *tmpl.Renderer
	globalSSE  httputil.Broadcaster
	sessionSSE httputil.SessionBroadcaster
}

// New creates a new Handler.
func New(
	mgr *manager.Manager,
	auth *httputil.AuthHandler,
	renderer *tmpl.Renderer,
	globalSSE httputil.Broadcaster,
	sessionSSE httputil.SessionBroadcaster,
) *Handler {
	return &Handler{
		manager:    mgr,
		auth:       auth,
		renderer:   renderer,
		globalSSE:  globalSSE,
		sessionSSE: sessionSSE,
	}
}

// Home serves the home page.
func (h *Handler) Home(w http.ResponseWriter, r *http.Request) {
	if r.URL.Path != "/" {
		http.NotFound(w, r)
		return
	}

	username := h.auth.GetUsername(r)
	if username == "" {
		h.renderer.RenderHTTP(w, "login", nil)
		return
	}

	data := map[string]interface{}{
		"Username": username,
		"Sessions": h.manager.GetSessions(),
		"LocalID":  h.manager.LocalID(),
	}
	h.renderer.RenderHTTP(w, "home", data)
}

// Sessions handles listing (GET) and creating (POST) sessions.
func (h *Handler) Sessions(w http.ResponseWriter, r *http.Request) {
	if r.Method == http.MethodPost {
		h.createSession(w, r)
		return
	}

	// GET - return session list partial for HTMX
	h.renderer.RenderPartial(w, "session_list", h.manager.GetSessions())
}

func (h *Handler) createSession(w http.ResponseWriter, r *http.Request) {
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

	info, err := h.manager.CreateSession(r.Context(), sessionType, destination, enableMLS)
	if err != nil {
		log.Printf("Failed to create session: %v", err)
		http.Error(w, "Failed to create session: "+err.Error(), http.StatusInternalServerError)
		return
	}

	log.Printf("Created %s session: %s -> %s", sessionType, info.ID, destination)
	http.Redirect(w, r, "/session/"+info.ID, http.StatusSeeOther)
}

// SessionRoutes routes session-specific requests.
func (h *Handler) SessionRoutes(w http.ResponseWriter, r *http.Request) {
	path := strings.TrimPrefix(r.URL.Path, "/session/")
	parts := strings.Split(path, "/")
	sessionID := parts[0]

	if sessionID == "" {
		http.Redirect(w, r, "/", http.StatusSeeOther)
		return
	}

	if len(parts) == 1 {
		if r.Method == http.MethodDelete {
			h.deleteSession(w, r, sessionID)
		} else {
			h.sessionDetail(w, r, sessionID)
		}
		return
	}

	action := parts[1]
	switch action {
	case "invite":
		h.invite(w, r, sessionID)
	case "remove":
		h.remove(w, r, sessionID)
	case "send":
		h.sendMessage(w, r, sessionID)
	case "messages":
		h.listMessages(w, r, sessionID)
	case "events":
		h.sessionSSEHandler(w, r, sessionID)
	case "delete":
		h.deleteSession(w, r, sessionID)
	default:
		http.NotFound(w, r)
	}
}

func (h *Handler) sessionDetail(w http.ResponseWriter, r *http.Request, sessionID string) {
	username := h.auth.GetUsername(r)
	if username == "" {
		http.Redirect(w, r, "/", http.StatusSeeOther)
		return
	}

	info, exists := h.manager.GetSession(sessionID)
	if !exists {
		http.Error(w, "Session not found", http.StatusNotFound)
		return
	}

	data := map[string]interface{}{
		"Username": username,
		"Session":  info,
		"Messages": h.manager.GetMessages(sessionID),
		"Clients":  h.manager.GetClients(),
		"LocalID":  h.manager.LocalID(),
	}
	h.renderer.RenderHTTP(w, "session_detail", data)
}

func (h *Handler) deleteSession(w http.ResponseWriter, r *http.Request, sessionID string) {
	if err := h.manager.DeleteSession(r.Context(), sessionID); err != nil {
		http.Error(w, "Failed to delete session: "+err.Error(), http.StatusInternalServerError)
		return
	}

	log.Printf("Deleted session: %s", sessionID)

	if r.Header.Get("HX-Request") == "true" {
		w.WriteHeader(http.StatusOK)
		return
	}

	http.Redirect(w, r, "/", http.StatusSeeOther)
}

func (h *Handler) invite(w http.ResponseWriter, r *http.Request, sessionID string) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	participant := strings.TrimSpace(r.FormValue("participant"))
	if participant == "" {
		http.Error(w, "Participant required", http.StatusBadRequest)
		return
	}

	if err := h.manager.InviteParticipant(r.Context(), sessionID, participant); err != nil {
		http.Error(w, "Failed to invite: "+err.Error(), http.StatusInternalServerError)
		return
	}

	log.Printf("Invited %s to session %s", participant, sessionID)

	info, _ := h.manager.GetSession(sessionID)
	h.renderer.RenderPartial(w, "participants", info)
}

func (h *Handler) remove(w http.ResponseWriter, r *http.Request, sessionID string) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	participant := strings.TrimSpace(r.FormValue("participant"))
	if participant == "" {
		http.Error(w, "Participant required", http.StatusBadRequest)
		return
	}

	if err := h.manager.RemoveParticipant(r.Context(), sessionID, participant); err != nil {
		http.Error(w, "Failed to remove: "+err.Error(), http.StatusInternalServerError)
		return
	}

	log.Printf("Removed %s from session %s", participant, sessionID)

	info, _ := h.manager.GetSession(sessionID)
	h.renderer.RenderPartial(w, "participants", info)
}

func (h *Handler) sendMessage(w http.ResponseWriter, r *http.Request, sessionID string) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	text := strings.TrimSpace(r.FormValue("message"))
	if text == "" {
		w.WriteHeader(http.StatusOK)
		return
	}

	if err := h.manager.SendMessage(r.Context(), sessionID, text); err != nil {
		http.Error(w, "Failed to send: "+err.Error(), http.StatusInternalServerError)
		return
	}

	w.WriteHeader(http.StatusOK)
}

func (h *Handler) listMessages(w http.ResponseWriter, r *http.Request, sessionID string) {
	messages := h.manager.GetMessages(sessionID)
	h.renderer.RenderPartial(w, "messages", messages)
}

func (h *Handler) sessionSSEHandler(w http.ResponseWriter, r *http.Request, sessionID string) {
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
	events := h.sessionSSE.Subscribe(r.Context(), sessionID, subID)

	// Send existing messages first
	messages := h.manager.GetMessages(sessionID)
	for _, msg := range messages {
		html := formatMessageHTML(msg)
		fmt.Fprintf(w, "event: message\ndata: %s\n\n", html)
	}
	flusher.Flush()

	// Stream new events
	for {
		select {
		case event, ok := <-events:
			if !ok {
				return
			}
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

// GlobalSSE handles global SSE connections.
func (h *Handler) GlobalSSE(w http.ResponseWriter, r *http.Request) {
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
	events := h.globalSSE.Subscribe(r.Context(), subID)

	// Send initial ping
	fmt.Fprintf(w, "event: ping\ndata: connected\n\n")
	flusher.Flush()

	// Stream events
	for {
		select {
		case event, ok := <-events:
			if !ok {
				return
			}
			data, _ := json.Marshal(event.Data)
			fmt.Fprintf(w, "event: %s\ndata: %s\n\n", event.Type, string(data))
			flusher.Flush()
		case <-r.Context().Done():
			return
		}
	}
}

// Clients returns known clients as JSON or datalist options.
func (h *Handler) Clients(w http.ResponseWriter, r *http.Request) {
	clients := h.manager.GetClients()

	if r.Header.Get("Accept") == "application/json" {
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(clients)
		return
	}

	h.renderer.RenderPartial(w, "clients_datalist", clients)
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
