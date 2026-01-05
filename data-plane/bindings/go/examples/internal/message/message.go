// Package message provides the Message model and storage for SLIM example applications.
package message

import (
	"sync"
	"time"
)

// Direction indicates whether a message was sent or received.
type Direction string

const (
	// DirectionSent indicates the message was sent by the local user.
	DirectionSent Direction = "sent"
	// DirectionReceived indicates the message was received from another participant.
	DirectionReceived Direction = "received"
)

// Message represents a chat message in a session.
type Message struct {
	ID        string            `json:"id"`
	SessionID string            `json:"sessionId"`
	Direction Direction         `json:"direction"`
	Sender    string            `json:"sender"`
	Text      string            `json:"text"`
	Metadata  map[string]string `json:"metadata,omitempty"`
	Timestamp time.Time         `json:"timestamp"`
}

// Store defines the interface for message storage.
type Store interface {
	// Add adds a message to the store for a session.
	Add(sessionID string, msg Message)

	// Get returns all messages for a session.
	Get(sessionID string) []Message

	// Delete removes all messages for a session.
	Delete(sessionID string)

	// Count returns the number of messages for a session.
	Count(sessionID string) int
}

// InMemoryStore implements Store with an in-memory map.
type InMemoryStore struct {
	messages map[string][]Message
	mu       sync.RWMutex
}

// NewInMemoryStore creates a new in-memory message store.
func NewInMemoryStore() *InMemoryStore {
	return &InMemoryStore{
		messages: make(map[string][]Message),
	}
}

// Add adds a message to the store for a session.
func (s *InMemoryStore) Add(sessionID string, msg Message) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.messages[sessionID] = append(s.messages[sessionID], msg)
}

// Get returns a copy of all messages for a session.
func (s *InMemoryStore) Get(sessionID string) []Message {
	s.mu.RLock()
	defer s.mu.RUnlock()
	msgs := s.messages[sessionID]
	if msgs == nil {
		return nil
	}
	// Return a copy to prevent external modification
	result := make([]Message, len(msgs))
	copy(result, msgs)
	return result
}

// Delete removes all messages for a session.
func (s *InMemoryStore) Delete(sessionID string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	delete(s.messages, sessionID)
}

// Count returns the number of messages for a session.
func (s *InMemoryStore) Count(sessionID string) int {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return len(s.messages[sessionID])
}
