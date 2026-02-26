// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package tests

import (
	"context"
	"fmt"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	slim "github.com/agntcy/slim-bindings-go"
)

// TestHarness manages a pair of communicating apps for integration testing
type TestHarness struct {
	Sender       *slim.App
	Receiver     *slim.App
	SenderName   *slim.Name
	ReceiverName *slim.Name
	SharedSecret string
	t            *testing.T
	cancel       context.CancelFunc
	wg           sync.WaitGroup
}

// ReceivedMessage represents a message received by the test harness
type ReceivedMessage struct {
	Data        []byte
	PayloadType string
	Metadata    map[string]string
	SourceName  *slim.Name
	Timestamp   time.Time
}

// MessageCollector collects messages received by the receiver
type MessageCollector struct {
	mu       sync.Mutex
	messages []ReceivedMessage
	count    atomic.Int32
}

func (mc *MessageCollector) Add(msg ReceivedMessage) {
	mc.mu.Lock()
	defer mc.mu.Unlock()
	mc.messages = append(mc.messages, msg)
	mc.count.Add(1)
}

func (mc *MessageCollector) GetAll() []ReceivedMessage {
	mc.mu.Lock()
	defer mc.mu.Unlock()
	result := make([]ReceivedMessage, len(mc.messages))
	copy(result, mc.messages)
	return result
}

func (mc *MessageCollector) Count() int {
	return int(mc.count.Load())
}

func (mc *MessageCollector) WaitForCount(ctx context.Context, expected int, timeout time.Duration) bool {
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		if mc.Count() >= expected {
			return true
		}
		select {
		case <-ctx.Done():
			return false
		case <-time.After(50 * time.Millisecond):
			// Continue waiting
		}
	}
	return mc.Count() >= expected
}

// WaitForMessages waits until the expected number of messages have been collected
// and returns a snapshot of all messages. It fails the test if the timeout is reached.
func (mc *MessageCollector) WaitForMessages(t *testing.T, expected int, timeout time.Duration) []ReceivedMessage {
	t.Helper()

	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()

	if !mc.WaitForCount(ctx, expected, timeout) {
		t.Fatalf("expected at least %d messages, got %d within %v", expected, mc.Count(), timeout)
	}
	return mc.GetAll()
}

// SetupTestHarness creates a sender and receiver app that can communicate
func SetupTestHarness(t *testing.T, testName string) (*TestHarness, *MessageCollector) {
	t.Helper()

	slim.InitializeWithDefaults()

	ctx, cancel := context.WithCancel(context.Background())

	// Create unique names for this test
	senderName := slim.NewName("org", fmt.Sprintf("%s-sender", testName), "v1")
	receiverName := slim.NewName("org", fmt.Sprintf("%s-receiver", testName), "v1")

	// Use a shared secret (must be at least 32 bytes)
	sharedSecret := "test-harness-shared-secret-must-be-32-bytes-or-more!"

	// create shared secret provider and verifier
	senderIdentityProvider := slim.IdentityProviderConfigSharedSecret{
		Data: sharedSecret,
		Id:   senderName.String(),
	}

	senderIdentityVerifier := slim.IdentityVerifierConfigSharedSecret{
		Data: sharedSecret,
		Id:   senderName.String(),
	}

	// Create sender app
	sender, err := slim.NewApp(
		senderName,
		senderIdentityProvider,
		senderIdentityVerifier,
	)
	if err != nil {
		cancel()
		t.Fatalf("Failed to create sender app: %v", err)
	}

	// create shared secret provider and verifier
	receiverIdentityProvider := slim.IdentityProviderConfigSharedSecret{
		Data: sharedSecret,
		Id:   receiverName.String(),
	}

	receiverIdentityVerifier := slim.IdentityVerifierConfigSharedSecret{
		Data: sharedSecret,
		Id:   receiverName.String(),
	}

	// Create receiver app
	receiver, err := slim.NewApp(
		receiverName,
		receiverIdentityProvider,
		receiverIdentityVerifier,
	)
	if err != nil {
		sender.Destroy()
		cancel()
		t.Fatalf("Failed to create receiver app: %v", err)
	}

	harness := &TestHarness{
		Sender:       sender,
		Receiver:     receiver,
		SenderName:   senderName,
		ReceiverName: receiverName,
		SharedSecret: sharedSecret,
		t:            t,
		cancel:       cancel,
	}

	collector := &MessageCollector{
		messages: make([]ReceivedMessage, 0),
	}

	// Start receiver listener in background
	harness.wg.Add(1)
	go harness.runReceiverListener(ctx, collector)

	// Give receiver time to start listening
	time.Sleep(100 * time.Millisecond)

	return harness, collector
}

// runReceiverListener listens for incoming sessions and messages
func (h *TestHarness) runReceiverListener(ctx context.Context, collector *MessageCollector) {
	defer h.wg.Done()

	h.t.Logf("[Receiver] Starting listener...")

	for {
		select {
		case <-ctx.Done():
			h.t.Logf("[Receiver] Listener stopped")
			return
		default:
			// Listen for incoming session with short timeout
			timeout := time.Millisecond * 100
			session, err := h.Receiver.ListenForSession(&timeout)
			if err != nil {
				// Timeout is expected, continue
				continue
			}

			if session == nil {
				continue
			}

			source, err := session.Source()
			if err != nil {
				h.t.Logf("[Receiver] Failed to get session source: %v", err)
				session.Destroy()
				continue
			}

			h.t.Logf("[Receiver] Received new session from %v", source.Components())

			// Handle this session in a separate goroutine
			h.wg.Add(1)
			go h.handleReceiverSession(ctx, session, collector)
		}
	}
}

// handleReceiverSession processes messages from a single session
func (h *TestHarness) handleReceiverSession(ctx context.Context, session *slim.Session, collector *MessageCollector) {
	defer h.wg.Done()
	defer session.Destroy()

	sessionID, err := session.SessionId()
	if err != nil {
		h.t.Logf("[Receiver] Failed to get session ID: %v", err)
		return
	}
	h.t.Logf("[Receiver] Handling session %d", sessionID)

	for {
		select {
		case <-ctx.Done():
			return
		default:
			// Try to get a message with short timeout
			timeout := time.Millisecond * 100
			receivedMsg, err := session.GetMessage(&timeout)
			if err != nil {
				// Timeout or error
				continue
			}

			// Extract message data from ReceivedMessage
			received := ReceivedMessage{
				Data:        receivedMsg.Payload,
				PayloadType: receivedMsg.Context.PayloadType,
				Metadata:    receivedMsg.Context.Metadata,
				SourceName:  receivedMsg.Context.SourceName,
				Timestamp:   time.Now(),
			}

			h.t.Logf("[Receiver] Received message: %d bytes, type=%s",
				len(received.Data), received.PayloadType)

			collector.Add(received)

			// Optionally send acknowledgment
			ackData := []byte("ACK")
			ackType := "text/plain"
			err = session.PublishToAndWait(receivedMsg.Context, ackData, &ackType, nil)
			if err != nil {
				h.t.Logf("[Receiver] Failed to send ACK: %v", err)
			} else {
				h.t.Logf("[Receiver] Sent ACK")
			}
		}
	}
}

// CreateSession creates a session from sender to receiver
func (h *TestHarness) CreateSession() (*slim.Session, error) {
	h.t.Helper()

	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypePointToPoint,
		EnableMls:   false,
	}

	h.t.Logf("[Sender] Creating session to %v...", h.ReceiverName.Components())
	session, err := h.Sender.CreateSessionAndWait(sessionConfig, h.ReceiverName)
	if err != nil {
		return nil, fmt.Errorf("failed to create session: %w", err)
	}

	sessionID, err := session.SessionId()
	if err != nil {
		session.Destroy()
		return nil, fmt.Errorf("failed to get session ID: %w", err)
	}

	h.t.Logf("[Sender] Session created: %d", sessionID)
	return session, nil
}

// SendMessage sends a message through the session
func (h *TestHarness) SendMessage(session *slim.Session, data []byte, payloadType *string, metadata *map[string]string) error {
	h.t.Helper()

	h.t.Logf("[Sender] Sending message: %d bytes", len(data))
	err := session.PublishAndWait(data, payloadType, metadata)
	if err != nil {
		return fmt.Errorf("failed to send message: %w", err)
	}

	h.t.Logf("[Sender] Message sent")
	return nil
}

// SendMessageWithCompletion sends a message and waits for delivery confirmation
func (h *TestHarness) SendMessageWithCompletion(session *slim.Session, data []byte, payloadType *string, metadata *map[string]string) error {
	h.t.Helper()

	h.t.Logf("[Sender] Sending message with completion: %d bytes", len(data))
	err := session.PublishAndWait(data, payloadType, metadata)
	if err != nil {
		return fmt.Errorf("failed to send message: %w", err)
	}

	h.t.Logf("[Sender] Message delivered successfully")
	return nil
}

// MustCreateSession creates a session and fails the test on error.
func (h *TestHarness) MustCreateSession() *slim.Session {
	h.t.Helper()

	session, err := h.CreateSession()
	if err != nil {
		h.t.Fatalf("Failed to create session: %v", err)
	}
	return session
}

// MustSendMessage sends a message and fails the test on error.
func (h *TestHarness) MustSendMessage(session *slim.Session, data []byte, payloadType *string, metadata *map[string]string) {
	h.t.Helper()

	if err := h.SendMessage(session, data, payloadType, metadata); err != nil {
		h.t.Fatalf("Failed to send message: %v", err)
	}
}

// MustSendMessageWithCompletion sends a message with completion tracking and fails the test on error.
func (h *TestHarness) MustSendMessageWithCompletion(session *slim.Session, data []byte, payloadType *string, metadata *map[string]string) {
	h.t.Helper()

	if err := h.SendMessageWithCompletion(session, data, payloadType, metadata); err != nil {
		h.t.Fatalf("Failed to send message with completion: %v", err)
	}
}

// Cleanup tears down the test harness
func (h *TestHarness) Cleanup() {
	h.t.Helper()

	h.t.Logf("Cleaning up test harness...")

	// Stop background tasks
	if h.cancel != nil {
		h.cancel()
	}

	// Wait for goroutines to finish (with timeout)
	done := make(chan struct{})
	go func() {
		h.wg.Wait()
		close(done)
	}()

	select {
	case <-done:
		h.t.Logf("Background tasks stopped")
	case <-time.After(2 * time.Second):
		h.t.Logf("Warning: Background tasks did not stop within timeout")
	}

	// Destroy apps
	if h.Sender != nil {
		h.Sender.Destroy()
	}
	if h.Receiver != nil {
		h.Receiver.Destroy()
	}

	h.t.Logf("Cleanup complete")
}
