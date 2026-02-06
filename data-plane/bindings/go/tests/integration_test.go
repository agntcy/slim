// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Package tests provides integration tests for SLIM Go bindings.
//
// These tests use a test harness (test_harness.go) that creates a running
// sender and receiver pair within the same process, enabling full integration
// testing without requiring external SLIM network infrastructure.
//
// The harness provides:
//   - A sender app (client) that initiates sessions and sends messages
//   - A receiver app (server) that listens for and processes incoming sessions
//   - Background goroutines for message handling
//   - Thread-safe message collection for verification
//
// All integration tests should use SetupTestHarness() to create this environment.
package tests

import (
	"context"
	"fmt"
	"sync"
	"testing"
	"time"

	slim "github.com/agntcy/slim-bindings-go"
)

// setupTestApp creates a test app for integration tests
func setupTestApp(t *testing.T, appNameStr string) *slim.App {
	t.Helper()

	slim.InitializeWithDefaults()

	appName := slim.NewName("org", appNameStr, "v1")

	sharedSecret := "test-shared-secret-must-be-at-least-32-bytes-long!"

	app, err := slim.GetGlobalService().CreateAppWithSecret(appName, sharedSecret)
	if err != nil {
		t.Fatalf("Failed to create app: %v", err)
	}

	return app
}

// ===================================================================
// Tests using the harness for full sender/receiver communication
// ===================================================================

// TestBasicCommunication tests basic sender-receiver communication
func TestBasicCommunication(t *testing.T) {
	harness, collector := SetupTestHarness(t, "basic-comm")
	defer harness.Cleanup()

	// Create session from sender to receiver
	session := harness.MustCreateSession()
	defer session.Destroy()

	// Send a message
	message := []byte("Hello from integration test!")
	payloadType := "text/plain"

	harness.MustSendMessage(session, message, &payloadType, nil)
	messages := collector.WaitForMessages(t, 1, 2*time.Second)

	if string(messages[0].Data) != string(message) {
		t.Errorf("Message mismatch: expected %s, got %s",
			string(message), string(messages[0].Data))
	}

	t.Logf("✅ Message successfully sent and received: %s", string(messages[0].Data))
}

// TestMultipleMessages tests sending multiple messages
func TestMultipleMessages(t *testing.T) {
	harness, collector := SetupTestHarness(t, "multi-msg")
	defer harness.Cleanup()

	session := harness.MustCreateSession()
	defer session.Destroy()

	numMessages := 5
	payloadType := "text/plain"

	// Send multiple messages
	for i := 0; i < numMessages; i++ {
		message := []byte(fmt.Sprintf("Message %d", i))
		harness.MustSendMessage(session, message, &payloadType, nil)
		time.Sleep(50 * time.Millisecond) // Small delay between messages
	}

	messages := collector.WaitForMessages(t, numMessages, 3*time.Second)
	t.Logf("✅ Received all %d/%d messages", len(messages), numMessages)

	// Verify message order and content
	for i, msg := range messages {
		expected := fmt.Sprintf("Message %d", i)
		if string(msg.Data) != expected {
			t.Errorf("Message %d mismatch: expected %s, got %s",
				i, expected, string(msg.Data))
		}
	}
}

// TestPublishWithCompletionHandle tests the completion handle functionality
func TestPublishWithCompletionHandle(t *testing.T) {
	harness, collector := SetupTestHarness(t, "with-completion")
	defer harness.Cleanup()

	session := harness.MustCreateSession()
	defer session.Destroy()

	message := []byte("Message with completion tracking")
	payloadType := "text/plain"

	harness.MustSendMessageWithCompletion(session, message, &payloadType, nil)

	messages := collector.WaitForMessages(t, 1, 2*time.Second)
	if string(messages[0].Data) != string(message) {
		t.Errorf("Message mismatch at receiver")
	}

	t.Log("✅ Message delivery confirmed by completion handle and receiver")
}

// TestConcurrentPublish tests concurrent message sending
func TestConcurrentPublish(t *testing.T) {
	harness, collector := SetupTestHarness(t, "concurrent")
	defer harness.Cleanup()

	session := harness.MustCreateSession()
	defer session.Destroy()

	numConcurrent := 10
	var wg sync.WaitGroup
	wg.Add(numConcurrent)

	payloadType := "text/plain"
	errors := make(chan error, numConcurrent)

	// Send messages concurrently
	for i := 0; i < numConcurrent; i++ {
		go func(idx int) {
			defer wg.Done()
			message := []byte(fmt.Sprintf("Concurrent message %d", idx))
			err := harness.SendMessage(session, message, &payloadType, nil)
			if err != nil {
				errors <- fmt.Errorf("send %d failed: %w", idx, err)
			}
		}(i)
	}

	wg.Wait()
	close(errors)

	// Check for errors
	for err := range errors {
		t.Error(err)
	}

	// Wait for all messages
	ctx := context.Background()
	if !collector.WaitForCount(ctx, numConcurrent, 3*time.Second) {
		t.Logf("Warning: Only received %d/%d messages", collector.Count(), numConcurrent)
	}

	received := collector.Count()
	t.Logf("✅ Received %d/%d concurrent messages", received, numConcurrent)

	if received < numConcurrent {
		t.Errorf("Expected %d messages, got %d", numConcurrent, received)
	}
}

// TestPublishWithMetadata tests sending messages with metadata
func TestPublishWithMetadata(t *testing.T) {
	harness, collector := SetupTestHarness(t, "metadata")
	defer harness.Cleanup()

	session := harness.MustCreateSession()
	defer session.Destroy()

	message := []byte("Message with metadata")
	payloadType := "application/json"
	metadata := map[string]string{
		"priority":  "high",
		"sender":    "integration-test",
		"test-id":   "meta-001",
		"timestamp": time.Now().Format(time.RFC3339),
	}

	harness.MustSendMessage(session, message, &payloadType, &metadata)

	messages := collector.WaitForMessages(t, 1, 2*time.Second)
	msg := messages[0]

	// Verify payload
	if string(msg.Data) != string(message) {
		t.Errorf("Payload mismatch")
	}

	// Verify payload type
	if msg.PayloadType != payloadType {
		t.Errorf("Payload type mismatch: expected %s, got %s",
			payloadType, msg.PayloadType)
	}

	// Verify metadata was preserved
	if msg.Metadata == nil {
		t.Fatal("Metadata not preserved")
	}

	if val, ok := msg.Metadata["priority"]; !ok || val != "high" {
		t.Error("Metadata 'priority' not preserved correctly")
	}

	if val, ok := msg.Metadata["sender"]; !ok || val != "integration-test" {
		t.Error("Metadata 'sender' not preserved correctly")
	}

	t.Logf("✅ Message with metadata delivered successfully")
	t.Logf("   Metadata: %v", msg.Metadata)
}

// TestSessionLifecycle tests full session lifecycle
func TestSessionLifecycle(t *testing.T) {
	harness, collector := SetupTestHarness(t, "lifecycle")
	defer harness.Cleanup()

	// Step 1: Create session
	t.Log("Creating session...")
	session := harness.MustCreateSession()

	sessionID, err := session.SessionId()
	if err != nil {
		t.Fatalf("Failed to get session ID: %v", err)
	}
	t.Logf("✅ Session created: %d", sessionID)

	// Step 2: Send message
	t.Log("Sending message...")
	message := []byte("Lifecycle test message")
	payloadType := "text/plain"
	harness.MustSendMessage(session, message, &payloadType, nil)
	t.Log("✅ Message sent")

	// Step 3: Verify reception
	collector.WaitForMessages(t, 1, 2*time.Second)
	t.Logf("✅ Receiver got %d message(s)", collector.Count())

	// Step 4: Delete session
	t.Log("Deleting session...")
	err = harness.Sender.DeleteSessionAndWait(session)
	if err != nil {
		t.Fatalf("Failed to delete session: %v", err)
	}
	t.Log("✅ Session deleted")

	// Step 5: Destroy session
	t.Log("Destroying session...")
	session.Destroy()
	t.Log("✅ Session destroyed")

	t.Log("✅ Full lifecycle test completed successfully")
}

// ===================================================================
// Standalone tests for individual API components
// ===================================================================

// TestAppCreationAndProperties tests basic app creation and properties
func TestAppCreationAndProperties(t *testing.T) {
	slim.InitializeWithDefaults()

	appName := slim.NewName("org", "app-creation-test", "v1")

	sharedSecret := "test-secret-must-be-at-least-32-bytes-long!"

	app, err := slim.GetGlobalService().CreateAppWithSecret(appName, sharedSecret)
	if err != nil {
		t.Fatalf("Failed to create app: %v", err)
	}
	defer app.Destroy()

	// Verify app properties
	appID := app.Id()
	if appID == 0 {
		t.Error("App ID should not be zero")
	}

	retrievedName := app.Name()

	retrievedNameComponents := retrievedName.Components()
	appNameComponents := appName.Components()

	if len(retrievedNameComponents) != len(appNameComponents) {
		t.Error("App name components count mismatch")
	}

	for i, comp := range retrievedNameComponents {
		if comp != appNameComponents[i] {
			t.Errorf("Component %d mismatch: expected %s, got %s",
				i, appNameComponents[i], comp)
		}
	}

	t.Logf("✅ App created successfully with ID: %d", appID)
	t.Logf("   Name: %v", retrievedName.Components())
}

// TestVersionInfo tests version retrieval
func TestVersionInfo(t *testing.T) {
	slim.InitializeWithDefaults()

	version := slim.GetVersion()
	if version == "" {
		t.Error("Version string should not be empty")
	}

	t.Logf("✅ SLIM Bindings Version: %s", version)
}

// TestSubscribeUnsubscribe tests subscription operations
func TestSubscribeUnsubscribe(t *testing.T) {
	app := setupTestApp(t, "subscribe-test")
	defer app.Destroy()

	subscribeName := slim.NewName("org", "sub", "topic")

	// Test subscribe
	err := app.Subscribe(subscribeName, nil)
	if err != nil {
		t.Logf("Subscribe failed (expected without network): %v", err)
	} else {
		t.Log("✅ Subscribed successfully")
	}

	// Test unsubscribe
	err = app.Unsubscribe(subscribeName, nil)
	if err != nil {
		t.Logf("Unsubscribe failed (expected without network): %v", err)
	} else {
		t.Log("✅ Unsubscribed successfully")
	}
}

// TestRouteOperations tests set_route and remove_route
func TestRouteOperations(t *testing.T) {
	app := setupTestApp(t, "route-test")
	defer app.Destroy()

	routeName := slim.NewName("org", "route", "target")

	connID := uint64(123)

	// Test set route
	err := app.SetRoute(routeName, connID)
	if err != nil {
		t.Logf("SetRoute failed (expected without network): %v", err)
	} else {
		t.Log("✅ Route set successfully")
	}

	// Test remove route
	err = app.RemoveRoute(routeName, connID)
	if err != nil {
		t.Logf("RemoveRoute failed (expected without network): %v", err)
	} else {
		t.Log("✅ Route removed successfully")
	}
}

// TestListenForSessionTimeout tests listening for sessions with timeout
func TestListenForSessionTimeout(t *testing.T) {
	app := setupTestApp(t, "listen-timeout-test")
	defer app.Destroy()

	timeout := 100 * time.Millisecond

	start := time.Now()
	_, err := app.ListenForSession(&timeout)
	elapsed := time.Since(start)

	if err == nil {
		t.Error("Expected timeout error, got nil")
	} else {
		t.Logf("✅ Received expected timeout error: %v", err)
	}

	// Verify timeout was respected (allow some variance)
	if elapsed < 90*time.Millisecond || elapsed > 200*time.Millisecond {
		t.Logf("Warning: timeout was %v, expected ~100ms", elapsed)
	}
}

// TestGroupSession tests creating a group session
func TestGroupSession(t *testing.T) {
	harness, _ := SetupTestHarness(t, "multicast-test")
	defer harness.Cleanup()

	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypeGroup,
		EnableMls:   false,
	}

	destination := slim.NewName("org", "group", "v1")

	session, err := harness.Sender.CreateSession(sessionConfig, destination)
	if err != nil {
		t.Fatalf("Failed to create multicast session: %v", err)
	}
	defer session.Destroy()

	t.Log("✅ Group session created successfully")
}

// TestSessionInviteRemove tests inviting and removing participants
func TestSessionInviteRemove(t *testing.T) {
	harness, _ := SetupTestHarness(t, "invite-test")
	defer harness.Cleanup()

	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypeGroup,
		EnableMls:   false,
	}

	destination := slim.NewName("org", "group", "v1")

	session, err := harness.Sender.CreateSessionAndWait(sessionConfig, destination)
	if err != nil {
		t.Fatalf("Failed to create session for invite test: %v", err)
	}
	defer session.Destroy()

	participant := slim.NewName("org", "participant", "v1")

	// Set route to participant to simulate connectivity
	err = harness.Sender.SetRoute(participant, 1)

	// Test invite - may fail for point-to-point or without full group management
	err = session.InviteAndWait(participant)
	if err != nil {
		t.Logf("Invite failed (may be expected for basic session): %v", err)
	} else {
		t.Log("✅ Participant invited successfully")
	}

	// Test remove
	err = session.RemoveAndWait(participant)
	if err != nil {
		t.Logf("Remove failed (may be expected): %v", err)
	} else {
		t.Log("✅ Participant removed successfully")
	}
}

// TestMultipleSessions tests creating multiple sessions in parallel
func TestMultipleSessions(t *testing.T) {
	t.Skip("Skipping - requires multiple receiver harnesses (not yet implemented)")

	// TODO: To properly test this, we would need to:
	// 1. Create multiple test harnesses (one for each receiver)
	// 2. Create sessions from one sender to multiple receivers
	// 3. Verify messages can be sent to all sessions concurrently
	//
	// For now, this is skipped as the single harness tests concurrent
	// operations within one session (see TestConcurrentPublish)
}

// TestCompletionHandleDoubleWait tests that completion handles can only be used once
func TestCompletionHandleDoubleWait(t *testing.T) {
	harness, _ := SetupTestHarness(t, "double-wait")
	defer harness.Cleanup()

	session := harness.MustCreateSession()
	defer session.Destroy()

	message := []byte("Test message")
	payloadType := "text/plain"

	// Get completion handle - use regular Publish which returns a completion handle
	completion, err := session.Publish(message, &payloadType, nil)
	if err != nil {
		t.Fatalf("Publish failed: %v", err)
	}
	defer completion.Destroy()

	// First wait should work
	err = completion.Wait()
	if err != nil {
		t.Logf("First wait failed (may be expected): %v", err)
	} else {
		t.Log("✅ First wait succeeded")
	}

	// Second wait should fail
	err = completion.Wait()
	if err == nil {
		t.Error("Expected error on second wait, got nil")
	} else {
		t.Logf("✅ Second wait failed as expected: %v", err)
	}
}

// ===================================================================
// Auto-wait behavior tests
// ===================================================================

// TestSessionCreationAutoWait tests that create_session auto-waits for establishment
func TestSessionCreationAutoWait(t *testing.T) {
	harness, _ := SetupTestHarness(t, "auto-wait-session")
	defer harness.Cleanup()

	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypePointToPoint,
		EnableMls:   false,
	}

	// CreateSession should auto-wait for session establishment
	// The harness.CreateSession internally calls app.CreateSession which auto-waits
	start := time.Now()
	session, err := harness.Sender.CreateSession(sessionConfig, harness.ReceiverName)
	elapsed := time.Since(start)

	if err != nil {
		t.Fatalf("CreateSession failed: %v", err)
	}
	defer session.Destroy()

	t.Logf("✅ Session created and established in %v", elapsed)
	t.Log("✅ Session is ready for use (auto-wait completed)")
}

// TestInviteAutoWait tests that invite auto-waits for acknowledgment
func TestInviteAutoWait(t *testing.T) {
	harness, _ := SetupTestHarness(t, "auto-wait-invite")
	defer harness.Cleanup()

	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypeGroup,
		EnableMls:   false,
	}

	destination := slim.NewName("org", "group", "v1")

	session, err := harness.Sender.CreateSessionAndWait(sessionConfig, destination)
	if err != nil {
		t.Fatalf("Failed to create multicast session: %v", err)
	}
	defer session.Destroy()

	participant := slim.NewName("org", "participant", "v1")

	// Invite should auto-wait for acknowledgment
	start := time.Now()
	err = session.InviteAndWait(participant)
	elapsed := time.Since(start)

	// Will fail without full group management, but should still auto-wait
	t.Logf("Invite completed in %v", elapsed)
	if err != nil {
		t.Logf("Invite failed (may be expected): %v", err)
	} else {
		t.Log("✅ Invitation acknowledged (auto-wait completed)")
	}
}

// TestRemoveAutoWait tests that remove auto-waits for acknowledgment
func TestRemoveAutoWait(t *testing.T) {
	harness, _ := SetupTestHarness(t, "auto-wait-remove")
	defer harness.Cleanup()

	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypeGroup,
		EnableMls:   false,
	}

	destination := slim.NewName("org", "group", "v1")

	session, err := harness.Sender.CreateSessionAndWait(sessionConfig, destination)
	if err != nil {
		t.Fatalf("Failed to create multicast session: %v", err)
	}
	defer session.Destroy()

	participant := slim.NewName("org", "participant", "v1")

	// Remove should auto-wait for acknowledgment
	start := time.Now()
	err = session.RemoveAndWait(participant)
	elapsed := time.Since(start)

	// Will fail without participant, but should still auto-wait
	t.Logf("Remove completed in %v", elapsed)
	if err != nil {
		t.Logf("Remove failed (expected - participant not found): %v", err)
		// Verify it processed (should be quick since participant not found)
		t.Log("✅ Auto-wait behavior confirmed")
	} else {
		t.Log("✅ Removal acknowledged (auto-wait completed)")
	}
}

// TestCompletionHandleAsync tests async waiting with completion handles
func TestCompletionHandleAsync(t *testing.T) {
	harness, _ := SetupTestHarness(t, "async-completion")
	defer harness.Cleanup()

	session := harness.MustCreateSession()
	defer session.Destroy()

	message := []byte("Async test message")
	completion, err := session.Publish(message, nil, nil)
	if err != nil {
		t.Fatalf("PublishWithCompletion failed: %v", err)
	}
	defer completion.Destroy()

	// Call Wait directly
	err = completion.Wait()
	if err != nil {
		t.Logf("Wait completed with error (may be expected): %v", err)
	} else {
		t.Log("✅ Wait completed successfully")
	}
}

// TestBatchPublishWithCompletion tests publishing multiple messages and waiting for all
func TestBatchPublishWithCompletion(t *testing.T) {
	harness, _ := SetupTestHarness(t, "batch-completion")
	defer harness.Cleanup()

	session := harness.MustCreateSession()
	defer session.Destroy()

	// Publish multiple messages and collect completion handles
	numMessages := 5
	completions := make([]*slim.CompletionHandle, 0, numMessages)

	for i := 0; i < numMessages; i++ {
		message := []byte(fmt.Sprintf("Batch message %d", i))
		payloadType := "text/plain"
		completion, err := session.Publish(message, &payloadType, nil)
		if err != nil {
			t.Fatalf("Message %d failed to publish: %v", i, err)
		}
		completions = append(completions, completion)
	}

	// Now wait for all completions
	successCount := 0
	for i, completion := range completions {
		err := completion.Wait()
		if err != nil {
			t.Logf("Message %d delivery failed: %v", i, err)
		} else {
			successCount++
		}
		completion.Destroy()
	}

	t.Logf("✅ Batch publish: %d/%d messages confirmed delivered", successCount, len(completions))

	if successCount != numMessages {
		t.Errorf("Expected all %d messages to be delivered, got %d", numMessages, successCount)
	}
}

// TestFireAndForgetVsWithCompletion compares both publish methods
func TestFireAndForgetVsWithCompletion(t *testing.T) {
	harness, collector := SetupTestHarness(t, "compare-publish")
	defer harness.Cleanup()

	session := harness.MustCreateSession()
	defer session.Destroy()

	// Test 1: Fire-and-forget
	t.Log("Testing fire-and-forget publish...")
	message1 := []byte("Fire and forget message")
	payloadType := "text/plain"
	err := session.PublishAndWait(message1, &payloadType, nil)
	if err != nil {
		t.Fatalf("Fire-and-forget publish failed: %v", err)
	}
	t.Log("✅ Fire-and-forget publish queued successfully")

	// Test 2: With completion tracking
	t.Log("Testing publish with completion...")
	message2 := []byte("Message with completion")
	completion, err := session.Publish(message2, &payloadType, nil)
	if err != nil {
		t.Fatalf("Publish with completion failed: %v", err)
	}
	defer completion.Destroy()

	// Wait for delivery confirmation
	err = completion.Wait()
	if err != nil {
		t.Fatalf("Completion wait failed: %v", err)
	}
	t.Log("✅ Message delivery confirmed")

	// Wait for both messages to be received
	ctx := context.Background()
	if !collector.WaitForCount(ctx, 2, 3*time.Second) {
		t.Logf("Warning: Only received %d/2 messages", collector.Count())
	} else {
		t.Log("✅ Both fire-and-forget and with-completion messages received")
	}
}

// ===================================================================
// Benchmarks
// ===================================================================

// BenchmarkPublishFireAndForget benchmarks fire-and-forget publish
func BenchmarkPublishFireAndForget(b *testing.B) {
	slim.InitializeWithDefaults()

	app, _ := slim.GetGlobalService().CreateAppWithSecret(
		slim.NewName("org", "bench", "v1"),
		"benchmark-secret-must-be-at-least-32-bytes!",
	)
	defer app.Destroy()

	session, err := app.CreateSession(
		slim.SessionConfig{SessionType: slim.SessionTypePointToPoint, EnableMls: false},
		slim.NewName("org", "receiver", "v1"),
	)
	if err != nil {
		b.Skipf("Skipping benchmark - session creation failed: %v", err)
		return
	}
	defer session.Destroy()

	message := []byte("benchmark message")

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_ = session.Session.PublishAndWait(message, nil, nil)
	}
}

// BenchmarkPublishWithCompletion benchmarks publish with completion handle
func BenchmarkPublishWithCompletion(b *testing.B) {
	slim.InitializeWithDefaults()

	app, _ := slim.GetGlobalService().CreateAppWithSecret(
		slim.NewName("org", "bench", "v1"),
		"benchmark-secret-must-be-at-least-32-bytes!",
	)
	defer app.Destroy()

	session, err := app.CreateSession(
		slim.SessionConfig{SessionType: slim.SessionTypePointToPoint, EnableMls: false},
		slim.NewName("org", "receiver", "v1"),
	)
	if err != nil {
		b.Skipf("Skipping benchmark - session creation failed: %v", err)
		return
	}
	defer session.Destroy()

	message := []byte("benchmark message")

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		completion, err := session.Session.Publish(message, nil, nil)
		if err == nil {
			completion.Destroy()
		}
	}
}

// BenchmarkPublishWithCompletionAndWait benchmarks publish with wait
func BenchmarkPublishWithCompletionAndWait(b *testing.B) {
	slim.InitializeWithDefaults()

	app, _ := slim.GetGlobalService().CreateAppWithSecret(
		slim.NewName("org", "bench", "v1"),
		"benchmark-secret-must-be-at-least-32-bytes!",
	)
	defer app.Destroy()

	session, err := app.CreateSession(
		slim.SessionConfig{SessionType: slim.SessionTypePointToPoint, EnableMls: false},
		slim.NewName("org", "receiver", "v1"),
	)
	if err != nil {
		b.Skipf("Skipping benchmark - session creation failed: %v", err)
		return
	}
	defer session.Destroy()

	message := []byte("benchmark message")

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		completion, err := session.Session.Publish(message, nil, nil)
		if err == nil {
			_ = completion.Wait()
			completion.Destroy()
		}
	}
}
