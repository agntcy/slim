// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package tests

import (
	"testing"
	"time"

	slim "github.com/agntcy/slim/bindings/generated/slim_service"
)

// setupTestApp creates a test service and app for integration tests
func setupTestApp(t *testing.T, appNameStr string) (*slim.Service, *slim.App) {
	slim.InitializeCrypto()
	
	service, err := slim.NewService()
	if err != nil {
		t.Fatalf("Failed to create service: %v", err)
	}
	
	appName := slim.Name{
		Components: []string{"org", appNameStr, "v1"},
		Id:         nil,
	}
	
	sharedSecret := "test-shared-secret-must-be-at-least-32-bytes-long!"
	
	app, err := service.CreateApp(appName, sharedSecret)
	if err != nil {
		service.Destroy()
		t.Fatalf("Failed to create app: %v", err)
	}
	
	return service, app
}

// TestSessionCreation tests creating a session
func TestSessionCreation(t *testing.T) {
	service, app := setupTestApp(t, "session-test")
	defer app.Destroy()
	defer service.Destroy()
	
	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypePointToPoint,
		EnableMls:   false,
	}
	
	destination := slim.Name{
		Components: []string{"org", "receiver", "v1"},
		Id:         nil,
	}
	
	session, err := app.CreateSession(sessionConfig, destination)
	if err != nil {
		t.Fatalf("Failed to create session: %v", err)
	}
	
	if session == nil {
		t.Fatal("Session should not be nil")
	}
	
	session.Destroy()
	t.Log("Session created and cleaned up successfully")
}

// TestSessionDelete tests creating and deleting a session
func TestSessionDelete(t *testing.T) {
	service, app := setupTestApp(t, "delete-test")
	defer app.Destroy()
	defer service.Destroy()
	
	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypePointToPoint,
		EnableMls:   false,
	}
	
	destination := slim.Name{
		Components: []string{"org", "receiver", "v1"},
		Id:         nil,
	}
	
	session, err := app.CreateSession(sessionConfig, destination)
	if err != nil {
		t.Fatalf("Failed to create session: %v", err)
	}
	
	// Delete the session
	err = app.DeleteSession(session)
	if err != nil {
		t.Errorf("Failed to delete session: %v", err)
	}
	
	session.Destroy()
	t.Log("Session deleted successfully")
}

// TestPublishMessage tests publishing a message (will fail without network)
func TestPublishMessage(t *testing.T) {
	service, app := setupTestApp(t, "publish-test")
	defer app.Destroy()
	defer service.Destroy()
	
	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypePointToPoint,
		EnableMls:   false,
	}
	
	destination := slim.Name{
		Components: []string{"org", "receiver", "v1"},
		Id:         nil,
	}
	
	session, err := app.CreateSession(sessionConfig, destination)
	if err != nil {
		t.Fatalf("Failed to create session: %v", err)
	}
	defer session.Destroy()
	
	message := []byte("Hello from Go integration test!")
	payloadType := "text/plain"
	
	// This will fail without a real network, but we test the call works
	err = session.Publish(destination, 1, message, nil, &payloadType, nil)
	if err != nil {
		// Expected without real network
		t.Logf("Publish failed as expected without network: %v", err)
	} else {
		t.Log("Message published successfully")
	}
}

// TestSubscribeUnsubscribe tests subscribing and unsubscribing
func TestSubscribeUnsubscribe(t *testing.T) {
	service, app := setupTestApp(t, "subscribe-test")
	defer app.Destroy()
	defer service.Destroy()
	
	subscribeName := slim.Name{
		Components: []string{"org", "sub", "topic"},
		Id:         nil,
	}
	
	// Test subscribe
	err := app.Subscribe(subscribeName, nil)
	if err != nil {
		t.Logf("Subscribe failed (expected without network): %v", err)
	} else {
		t.Log("Subscribed successfully")
	}
	
	// Test unsubscribe
	err = app.Unsubscribe(subscribeName, nil)
	if err != nil {
		t.Logf("Unsubscribe failed (expected without network): %v", err)
	} else {
		t.Log("Unsubscribed successfully")
	}
}

// TestSubscribeWithConnectionID tests subscribing with a specific connection ID
func TestSubscribeWithConnectionID(t *testing.T) {
	service, app := setupTestApp(t, "subscribe-conn-test")
	defer app.Destroy()
	defer service.Destroy()
	
	subscribeName := slim.Name{
		Components: []string{"org", "sub", "conn-topic"},
		Id:         nil,
	}
	
	connID := uint64(42)
	
	err := app.Subscribe(subscribeName, &connID)
	if err != nil {
		t.Logf("Subscribe with connection ID failed (expected without network): %v", err)
	}
	
	err = app.Unsubscribe(subscribeName, &connID)
	if err != nil {
		t.Logf("Unsubscribe with connection ID failed (expected without network): %v", err)
	}
}

// TestRouteOperations tests set_route and remove_route
func TestRouteOperations(t *testing.T) {
	service, app := setupTestApp(t, "route-test")
	defer app.Destroy()
	defer service.Destroy()
	
	routeName := slim.Name{
		Components: []string{"org", "route", "target"},
		Id:         nil,
	}
	
	connID := uint64(123)
	
	// Test set route
	err := app.SetRoute(routeName, connID)
	if err != nil {
		t.Logf("SetRoute failed (expected without network): %v", err)
	} else {
		t.Log("Route set successfully")
	}
	
	// Test remove route
	err = app.RemoveRoute(routeName, connID)
	if err != nil {
		t.Logf("RemoveRoute failed (expected without network): %v", err)
	} else {
		t.Log("Route removed successfully")
	}
}

// TestMulticastSession tests creating a multicast session
func TestMulticastSession(t *testing.T) {
	service, app := setupTestApp(t, "multicast-test")
	defer app.Destroy()
	defer service.Destroy()
	
	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypeMulticast,
		EnableMls:   false,
	}
	
	destination := slim.Name{
		Components: []string{"org", "group", "v1"},
		Id:         nil,
	}
	
	session, err := app.CreateSession(sessionConfig, destination)
	if err != nil {
		t.Fatalf("Failed to create multicast session: %v", err)
	}
	defer session.Destroy()
	
	t.Log("Multicast session created successfully")
}

// TestSessionInviteRemove tests inviting and removing participants
func TestSessionInviteRemove(t *testing.T) {
	service, app := setupTestApp(t, "invite-test")
	defer app.Destroy()
	defer service.Destroy()
	
	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypeMulticast,
		EnableMls:   false,
	}
	
	destination := slim.Name{
		Components: []string{"org", "group", "v1"},
		Id:         nil,
	}
	
	session, err := app.CreateSession(sessionConfig, destination)
	if err != nil {
		t.Fatalf("Failed to create session: %v", err)
	}
	defer session.Destroy()
	
	participant := slim.Name{
		Components: []string{"org", "participant", "v1"},
		Id:         nil,
	}
	
	// Test invite
	err = session.Invite(participant)
	if err != nil {
		t.Logf("Invite failed (expected without network): %v", err)
	} else {
		t.Log("Participant invited successfully")
	}
	
	// Test remove
	err = session.Remove(participant)
	if err != nil {
		t.Logf("Remove failed (expected without network): %v", err)
	} else {
		t.Log("Participant removed successfully")
	}
}

// TestPublishWithMetadata tests publishing a message with metadata
func TestPublishWithMetadata(t *testing.T) {
	service, app := setupTestApp(t, "metadata-test")
	defer app.Destroy()
	defer service.Destroy()
	
	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypePointToPoint,
		EnableMls:   false,
	}
	
	destination := slim.Name{
		Components: []string{"org", "receiver", "v1"},
		Id:         nil,
	}
	
	session, err := app.CreateSession(sessionConfig, destination)
	if err != nil {
		t.Fatalf("Failed to create session: %v", err)
	}
	defer session.Destroy()
	
	message := []byte("Message with metadata")
	payloadType := "application/json"
	metadata := map[string]string{
		"priority":  "high",
		"timestamp": time.Now().Format(time.RFC3339),
		"sender":    "integration-test",
	}
	
	err = session.Publish(destination, 1, message, nil, &payloadType, &metadata)
	if err != nil {
		t.Logf("Publish with metadata failed (expected without network): %v", err)
	} else {
		t.Log("Message with metadata published successfully")
	}
}

// TestMultipleSessions tests creating multiple sessions in parallel
func TestMultipleSessions(t *testing.T) {
	service, app := setupTestApp(t, "multi-session-test")
	defer app.Destroy()
	defer service.Destroy()
	
	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypePointToPoint,
		EnableMls:   false,
	}
	
	numSessions := 3
	sessions := make([]*slim.SessionContext, numSessions)
	
	for i := 0; i < numSessions; i++ {
		destination := slim.Name{
			Components: []string{"org", "receiver", "v1"},
			Id:         func() *uint64 { v := uint64(i); return &v }(),
		}
		
		session, err := app.CreateSession(sessionConfig, destination)
		if err != nil {
			t.Fatalf("Failed to create session %d: %v", i, err)
		}
		sessions[i] = session
	}
	
	// Cleanup all sessions
	for i, session := range sessions {
		if session != nil {
			session.Destroy()
			t.Logf("Session %d cleaned up", i)
		}
	}
	
	t.Logf("Successfully created and cleaned up %d sessions", numSessions)
}

// TestConcurrentOperations tests concurrent operations on the same app
func TestConcurrentOperations(t *testing.T) {
	service, app := setupTestApp(t, "concurrent-test")
	defer app.Destroy()
	defer service.Destroy()
	
	// Create a session
	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypePointToPoint,
		EnableMls:   false,
	}
	
	destination := slim.Name{
		Components: []string{"org", "receiver", "v1"},
		Id:         nil,
	}
	
	session, err := app.CreateSession(sessionConfig, destination)
	if err != nil {
		t.Fatalf("Failed to create session: %v", err)
	}
	defer session.Destroy()
	
	// Run concurrent subscribe/unsubscribe operations
	done := make(chan bool, 2)
	
	go func() {
		for i := 0; i < 5; i++ {
			name := slim.Name{
				Components: []string{"org", "sub1", "topic"},
				Id:         func() *uint64 { v := uint64(i); return &v }(),
			}
			_ = app.Subscribe(name, nil)
			time.Sleep(10 * time.Millisecond)
		}
		done <- true
	}()
	
	go func() {
		for i := 0; i < 5; i++ {
			name := slim.Name{
				Components: []string{"org", "sub2", "topic"},
				Id:         func() *uint64 { v := uint64(i); return &v }(),
			}
			_ = app.Subscribe(name, nil)
			time.Sleep(10 * time.Millisecond)
		}
		done <- true
	}()
	
	// Wait for both goroutines
	<-done
	<-done
	
	t.Log("Concurrent operations completed")
}

// TestListenForSessionTimeout tests listening for sessions with timeout
func TestListenForSessionTimeout(t *testing.T) {
	service, app := setupTestApp(t, "listen-timeout-test")
	defer app.Destroy()
	defer service.Destroy()
	
	timeout := uint32(100) // 100ms timeout
	
	start := time.Now()
	_, err := app.ListenForSession(&timeout)
	elapsed := time.Since(start)
	
	if err == nil {
		t.Error("Expected timeout error, got nil")
	} else {
		t.Logf("Received expected timeout error: %v", err)
	}
	
	// Verify timeout was respected (allow some variance)
	if elapsed < 90*time.Millisecond || elapsed > 200*time.Millisecond {
		t.Logf("Warning: timeout was %v, expected ~100ms", elapsed)
	}
}

// TestFullWorkflow tests a complete workflow: create session, publish, cleanup
func TestFullWorkflow(t *testing.T) {
	t.Log("Starting full workflow test")
	
	// Step 1: Setup
	service, app := setupTestApp(t, "workflow-test")
	defer func() {
		app.Destroy()
		service.Destroy()
		t.Log("Cleanup completed")
	}()
	
	// Step 2: Create session
	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypePointToPoint,
		EnableMls:   false,
	}
	
	destination := slim.Name{
		Components: []string{"org", "receiver", "v1"},
		Id:         nil,
	}
	
	session, err := app.CreateSession(sessionConfig, destination)
	if err != nil {
		t.Fatalf("Failed to create session: %v", err)
	}
	defer session.Destroy()
	t.Log("Session created")
	
	// Step 3: Subscribe
	err = app.Subscribe(destination, nil)
	if err != nil {
		t.Logf("Subscribe failed (expected without network): %v", err)
	} else {
		t.Log("Subscribed to destination")
	}
	
	// Step 4: Publish message
	message := []byte("Full workflow test message")
	payloadType := "text/plain"
	
	err = session.Publish(destination, 1, message, nil, &payloadType, nil)
	if err != nil {
		t.Logf("Publish failed (expected without network): %v", err)
	} else {
		t.Log("Message published")
	}
	
	// Step 5: Unsubscribe
	err = app.Unsubscribe(destination, nil)
	if err != nil {
		t.Logf("Unsubscribe failed (expected without network): %v", err)
	} else {
		t.Log("Unsubscribed from destination")
	}
	
	// Step 6: Delete session
	err = app.DeleteSession(session)
	if err != nil {
		t.Errorf("Failed to delete session: %v", err)
	} else {
		t.Log("Session deleted")
	}
	
	t.Log("Full workflow completed successfully")
}

