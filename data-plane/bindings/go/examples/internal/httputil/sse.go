package httputil

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"sync"

	"github.com/agntcy/slim/bindings/go/examples/internal/config"
)

// SSEEvent represents a Server-Sent Event.
type SSEEvent struct {
	Type string      `json:"type"`
	Data interface{} `json:"data"`
}

// Broadcaster manages SSE subscriptions and broadcasts events.
type Broadcaster interface {
	// Subscribe creates a new subscription and returns an event channel.
	// The context is used to automatically unsubscribe when cancelled.
	Subscribe(ctx context.Context, id string) <-chan SSEEvent

	// Unsubscribe removes a subscription by ID.
	Unsubscribe(id string)

	// Broadcast sends an event to all subscribers.
	Broadcast(event SSEEvent)

	// SubscriberCount returns the number of active subscribers.
	SubscriberCount() int
}

type broadcaster struct {
	subscribers map[string]chan SSEEvent
	mu          sync.RWMutex
	bufferSize  int
}

// NewBroadcaster creates a new Broadcaster with the specified buffer size.
func NewBroadcaster(bufferSize int) Broadcaster {
	if bufferSize <= 0 {
		bufferSize = config.DefaultSSEBufferSize
	}
	return &broadcaster{
		subscribers: make(map[string]chan SSEEvent),
		bufferSize:  bufferSize,
	}
}

// Subscribe creates a new subscription.
func (b *broadcaster) Subscribe(ctx context.Context, id string) <-chan SSEEvent {
	b.mu.Lock()
	defer b.mu.Unlock()

	// Close existing subscription if any
	if existing, ok := b.subscribers[id]; ok {
		close(existing)
	}

	ch := make(chan SSEEvent, b.bufferSize)
	b.subscribers[id] = ch

	// Auto-unsubscribe when context is cancelled
	go func() {
		<-ctx.Done()
		b.Unsubscribe(id)
	}()

	return ch
}

// Unsubscribe removes a subscription.
func (b *broadcaster) Unsubscribe(id string) {
	b.mu.Lock()
	defer b.mu.Unlock()

	if ch, ok := b.subscribers[id]; ok {
		close(ch)
		delete(b.subscribers, id)
	}
}

// Broadcast sends an event to all subscribers.
// Events are dropped if a subscriber's buffer is full.
func (b *broadcaster) Broadcast(event SSEEvent) {
	b.mu.RLock()
	defer b.mu.RUnlock()

	for _, ch := range b.subscribers {
		select {
		case ch <- event:
		default:
			// Drop event if channel is full
		}
	}
}

// SubscriberCount returns the number of active subscribers.
func (b *broadcaster) SubscriberCount() int {
	b.mu.RLock()
	defer b.mu.RUnlock()
	return len(b.subscribers)
}

// SessionBroadcaster manages SSE subscriptions per session.
type SessionBroadcaster interface {
	// Subscribe creates a subscription for a session.
	Subscribe(ctx context.Context, sessionID, subscriberID string) <-chan SSEEvent

	// Unsubscribe removes a subscription.
	Unsubscribe(sessionID, subscriberID string)

	// Broadcast sends an event to all subscribers of a session.
	Broadcast(sessionID string, event SSEEvent)

	// DeleteSession removes all subscriptions for a session.
	DeleteSession(sessionID string)
}

type sessionBroadcaster struct {
	sessions   map[string]map[string]chan SSEEvent
	mu         sync.RWMutex
	bufferSize int
}

// NewSessionBroadcaster creates a new SessionBroadcaster.
func NewSessionBroadcaster(bufferSize int) SessionBroadcaster {
	if bufferSize <= 0 {
		bufferSize = config.DefaultSSEBufferSize
	}
	return &sessionBroadcaster{
		sessions:   make(map[string]map[string]chan SSEEvent),
		bufferSize: bufferSize,
	}
}

// Subscribe creates a subscription for a session.
func (b *sessionBroadcaster) Subscribe(ctx context.Context, sessionID, subscriberID string) <-chan SSEEvent {
	b.mu.Lock()
	defer b.mu.Unlock()

	if b.sessions[sessionID] == nil {
		b.sessions[sessionID] = make(map[string]chan SSEEvent)
	}

	// Close existing subscription if any
	if existing, ok := b.sessions[sessionID][subscriberID]; ok {
		close(existing)
	}

	ch := make(chan SSEEvent, b.bufferSize)
	b.sessions[sessionID][subscriberID] = ch

	// Auto-unsubscribe when context is cancelled
	go func() {
		<-ctx.Done()
		b.Unsubscribe(sessionID, subscriberID)
	}()

	return ch
}

// Unsubscribe removes a subscription.
func (b *sessionBroadcaster) Unsubscribe(sessionID, subscriberID string) {
	b.mu.Lock()
	defer b.mu.Unlock()

	if subs, ok := b.sessions[sessionID]; ok {
		if ch, ok := subs[subscriberID]; ok {
			close(ch)
			delete(subs, subscriberID)
		}
		if len(subs) == 0 {
			delete(b.sessions, sessionID)
		}
	}
}

// Broadcast sends an event to all subscribers of a session.
func (b *sessionBroadcaster) Broadcast(sessionID string, event SSEEvent) {
	b.mu.RLock()
	defer b.mu.RUnlock()

	if subs, ok := b.sessions[sessionID]; ok {
		for _, ch := range subs {
			select {
			case ch <- event:
			default:
				// Drop event if channel is full
			}
		}
	}
}

// DeleteSession removes all subscriptions for a session.
func (b *sessionBroadcaster) DeleteSession(sessionID string) {
	b.mu.Lock()
	defer b.mu.Unlock()

	if subs, ok := b.sessions[sessionID]; ok {
		for _, ch := range subs {
			close(ch)
		}
		delete(b.sessions, sessionID)
	}
}

// SSEHandler returns an http.HandlerFunc that streams SSE events.
func SSEHandler(b Broadcaster, getSubscriberID func(r *http.Request) string) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
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

		subID := getSubscriberID(r)
		events := b.Subscribe(r.Context(), subID)

		for {
			select {
			case event, ok := <-events:
				if !ok {
					return
				}
				data, err := json.Marshal(event.Data)
				if err != nil {
					continue
				}
				fmt.Fprintf(w, "event: %s\ndata: %s\n\n", event.Type, data)
				flusher.Flush()
			case <-r.Context().Done():
				return
			}
		}
	}
}

// SessionSSEHandler returns an http.HandlerFunc that streams SSE events for a session.
func SessionSSEHandler(b SessionBroadcaster, getSessionID, getSubscriberID func(r *http.Request) string) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
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

		sessionID := getSessionID(r)
		subID := getSubscriberID(r)
		events := b.Subscribe(r.Context(), sessionID, subID)

		for {
			select {
			case event, ok := <-events:
				if !ok {
					return
				}
				data, err := json.Marshal(event.Data)
				if err != nil {
					continue
				}
				fmt.Fprintf(w, "event: %s\ndata: %s\n\n", event.Type, data)
				flusher.Flush()
			case <-r.Context().Done():
				return
			}
		}
	}
}
