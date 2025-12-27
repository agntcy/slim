// Package handler provides HTTP handlers for sessionclient.
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
	"github.com/agntcy/slim/bindings/go/examples/sessionclient/internal/client"
)

// Handler provides HTTP request handlers for sessionclient.
type Handler struct {
	client     *client.Client
	auth       *httputil.AuthHandler
	renderer   *tmpl.Renderer
	globalSSE  httputil.Broadcaster
	sessionSSE httputil.SessionBroadcaster
}

// New creates a new Handler.
func New(
	c *client.Client,
	auth *httputil.AuthHandler,
	renderer *tmpl.Renderer,
	globalSSE httputil.Broadcaster,
	sessionSSE httputil.SessionBroadcaster,
) *Handler {
	return &Handler{
		client:     c,
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
		"Username":    username,
		"Invitations": h.client.GetPendingInvites(),
		"Sessions":    h.client.GetJoinedSessions(),
		"LocalID":     h.client.LocalID(),
	}
	h.renderer.RenderHTTP(w, "home", data)
}

// Invitations returns the invitations list partial.
func (h *Handler) Invitations(w http.ResponseWriter, r *http.Request) {
	h.renderer.RenderPartial(w, "invitations_list", h.client.GetPendingInvites())
}

// InvitationRoutes routes invitation-specific requests.
func (h *Handler) InvitationRoutes(w http.ResponseWriter, r *http.Request) {
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
		h.acceptInvitation(w, r, inviteID)
	case "decline":
		h.declineInvitation(w, r, inviteID)
	default:
		http.NotFound(w, r)
	}
}

func (h *Handler) acceptInvitation(w http.ResponseWriter, r *http.Request, inviteID string) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	if err := h.client.AcceptInvitation(r.Context(), inviteID); err != nil {
		log.Printf("Failed to accept invitation %s: %v", inviteID, err)
		http.Error(w, "Failed to accept: "+err.Error(), http.StatusInternalServerError)
		return
	}

	log.Printf("Accepted invitation: %s", inviteID)
	h.renderer.RenderPartial(w, "invitations_list", h.client.GetPendingInvites())
}

func (h *Handler) declineInvitation(w http.ResponseWriter, r *http.Request, inviteID string) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	if err := h.client.DeclineInvitation(r.Context(), inviteID); err != nil {
		log.Printf("Failed to decline invitation %s: %v", inviteID, err)
		http.Error(w, "Failed to decline: "+err.Error(), http.StatusInternalServerError)
		return
	}

	log.Printf("Declined invitation: %s", inviteID)
	h.renderer.RenderPartial(w, "invitations_list", h.client.GetPendingInvites())
}

// Sessions returns the sessions list partial.
func (h *Handler) Sessions(w http.ResponseWriter, r *http.Request) {
	h.renderer.RenderPartial(w, "sessions_list", h.client.GetJoinedSessions())
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
		h.sessionDetail(w, r, sessionID)
		return
	}

	action := parts[1]
	switch action {
	case "send":
		h.sendMessage(w, r, sessionID)
	case "leave":
		h.leaveSession(w, r, sessionID)
	case "events":
		h.sessionSSEHandler(w, r, sessionID)
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

	session, exists := h.client.GetSession(sessionID)
	if !exists {
		http.Error(w, "Session not found", http.StatusNotFound)
		return
	}

	data := map[string]interface{}{
		"Username": username,
		"Session":  session,
		"Messages": h.client.GetMessages(sessionID),
		"LocalID":  h.client.LocalID(),
	}
	h.renderer.RenderHTTP(w, "session_detail", data)
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

	if err := h.client.SendMessage(r.Context(), sessionID, text); err != nil {
		http.Error(w, "Failed to send: "+err.Error(), http.StatusInternalServerError)
		return
	}

	w.WriteHeader(http.StatusOK)
}

func (h *Handler) leaveSession(w http.ResponseWriter, r *http.Request, sessionID string) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	if err := h.client.LeaveSession(r.Context(), sessionID); err != nil {
		http.Error(w, "Failed to leave: "+err.Error(), http.StatusInternalServerError)
		return
	}

	log.Printf("Left session: %s", sessionID)

	if r.Header.Get("HX-Request") == "true" {
		w.Header().Set("HX-Redirect", "/")
		w.WriteHeader(http.StatusOK)
		return
	}

	http.Redirect(w, r, "/", http.StatusSeeOther)
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
	messages := h.client.GetMessages(sessionID)
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

	fmt.Fprintf(w, "event: ping\ndata: connected\n\n")
	flusher.Flush()

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
