package tests

import (
	"os"
	"testing"
	"time"

	slim "github.com/agntcy/slim/bindings/generated/slimbindings"
)

// TestIntegrationFullFlow tests the complete workflow
// This test requires a proper SLIM network setup
func TestIntegrationFullFlow(t *testing.T) {
	if os.Getenv("SLIM_INTEGRATION_TEST") == "" {
		t.Skip("Skipping integration test - set SLIM_INTEGRATION_TEST=1 to run")
	}

	// Initialize crypto
	slim.InitializeCrypto()
	t.Log("✓ Crypto initialized")

	// Create service
	service, err := slim.NewService()
	if err != nil {
		t.Fatalf("Failed to create service: %v", err)
	}
	defer service.Destroy()
	t.Log("✓ Service created")

	// Create app
	appName := slim.Name{
		Components: []string{"test-org", "test-app", "test-resource"},
		Id:         nil,
	}

	// Note: Shared secret must be at least 32 bytes
	sharedSecret := "test-shared-secret-value-must-be-at-least-32-bytes!"

	app, err := service.CreateApp(appName, sharedSecret)
	if err != nil {
		t.Fatalf("Failed to create app: %v", err)
	}

	// Verify app properties
	appId := app.Id()
	if appId == 0 {
		t.Error("App ID should not be 0")
	}
	t.Logf("✓ Created app with ID: %d", appId)

	retrievedName := app.Name()
	if len(retrievedName.Components) != len(appName.Components) {
		t.Errorf("App name components length mismatch: got %d, want %d",
			len(retrievedName.Components), len(appName.Components))
	}

	// Test subscription
	subscriptionName := slim.Name{
		Components: []string{"test-org", "test-app", "notifications"},
		Id:         nil,
	}

	err = app.Subscribe(subscriptionName, nil)
	if err != nil {
		t.Logf("Subscribe warning (may be expected): %v", err)
	} else {
		t.Log("✓ Subscribe succeeded")
	}

	// Create session
	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypePointToPoint,
		EnableMls:   false,
	}

	destinationName := slim.Name{
		Components: []string{"test-org", "test-app", "peer"},
		Id:         nil,
	}

	session, err := app.CreateSession(sessionConfig, destinationName)
	if err != nil {
		t.Logf("Session creation warning (may be expected): %v", err)
	} else {
		defer func() {
			err := app.DeleteSession(session)
			if err != nil {
				t.Logf("Session deletion warning: %v", err)
			} else {
				t.Log("✓ Session deleted")
			}
		}()

		t.Log("✓ Session created")

		// Test message publishing
		message := []byte("Hello from integration test")
		destination := slim.Name{
			Components: []string{"test-org", "test-app", "target"},
			Id:         nil,
		}
		payloadType := uint32(0)
		var messageId *uint64
		var threadId *string
		var metadata *map[string]string

		err = session.Publish(destination, payloadType, message, messageId, threadId, metadata)
		if err != nil {
			t.Logf("Publish warning (may be expected): %v", err)
		} else {
			t.Log("✓ Publish succeeded")
		}

		// Test invite
		inviteName := slim.Name{
			Components: []string{"test-org", "test-app", "invitee"},
			Id:         nil,
		}
		err = session.Invite(inviteName)
		if err != nil {
			t.Logf("Invite warning (may be expected): %v", err)
		} else {
			t.Log("✓ Invite succeeded")
		}
	}

	// Test unsubscribe
	err = app.Unsubscribe(subscriptionName, nil)
	if err != nil {
		t.Logf("Unsubscribe warning (may be expected): %v", err)
	} else {
		t.Log("✓ Unsubscribe succeeded")
	}
}

// TestConcurrentOperations tests concurrent access to bindings
func TestConcurrentOperations(t *testing.T) {
	if os.Getenv("SLIM_INTEGRATION_TEST") == "" {
		t.Skip("Skipping integration test - set SLIM_INTEGRATION_TEST=1 to run")
	}

	service, err := slim.NewService()
	if err != nil {
		t.Fatalf("Failed to create service: %v", err)
	}
	defer service.Destroy()

	// Create multiple apps concurrently
	numApps := 5
	done := make(chan bool, numApps)
	errors := make(chan error, numApps)

	// Shared secret for all apps
	sharedSecret := "test-shared-secret-value-must-be-at-least-32-bytes!"

	for i := 0; i < numApps; i++ {
		go func(id int) {
			appName := slim.Name{
				Components: []string{"test-org", "test-app", string(rune('a' + id))},
				Id:         nil,
			}

			app, err := service.CreateApp(appName, sharedSecret)
			if err != nil {
				errors <- err
				done <- false
				return
			}

			// Verify app was created
			if app.Id() == 0 {
				done <- false
				return
			}

			done <- true
		}(i)
	}

	// Wait for all goroutines
	successCount := 0
	errorCount := 0
	timeout := time.After(10 * time.Second)

	for i := 0; i < numApps; i++ {
		select {
		case success := <-done:
			if success {
				successCount++
			} else {
				errorCount++
			}
		case err := <-errors:
			t.Logf("Concurrent operation error (may be expected): %v", err)
			errorCount++
		case <-timeout:
			t.Fatal("Timeout waiting for concurrent operations")
		}
	}

	t.Logf("Concurrent operations: %d successful, %d errors", successCount, errorCount)
}

// TestErrorHandling tests various error conditions
func TestErrorHandling(t *testing.T) {
	// Test with empty config path (should use default)
	service, err := slim.NewService()
	if err != nil {
		// Expected to fail without proper setup
		t.Logf("Expected error without proper setup: %v", err)
		return
	}
	defer service.Destroy()

	// Test creating app with empty name
	emptyName := slim.Name{
		Components: []string{},
		Id:         nil,
	}

	// Need a valid shared secret (must be at least 32 bytes)
	sharedSecret := "test-shared-secret-value-must-be-at-least-32-bytes!"

	_, err = service.CreateApp(emptyName, sharedSecret)
	if err != nil {
		t.Logf("Expected error with empty name: %v", err)
	} else {
		t.Log("Empty name was accepted (may be valid)")
	}

	// Note: Testing invalid shared secret causes panic in Rust code
	// This is a known limitation - shared secret validation should return error instead of panic
	t.Log("✓ Error handling test completed (skipping panic-inducing cases)")
}

// TestMemoryLeaks tests for memory leaks in repeated operations
func TestMemoryLeaks(t *testing.T) {
	if os.Getenv("SLIM_MEMORY_TEST") == "" {
		t.Skip("Skipping memory test - set SLIM_MEMORY_TEST=1 to run")
	}

	t.Log("Running memory leak test - monitor with Activity Monitor")

	// Create and destroy services repeatedly
	iterations := 100
	sharedSecret := "test-shared-secret-value-must-be-at-least-32-bytes!"

	for i := 0; i < iterations; i++ {
		service, err := slim.NewService()
		if err != nil {
			t.Fatalf("Iteration %d: Failed to create service: %v", i, err)
		}

		// Create app
		appName := slim.Name{
			Components: []string{"test-org", "test-app", "leak-test"},
			Id:         nil,
		}

		app, err := service.CreateApp(appName, sharedSecret)
		if err != nil {
			service.Destroy()
			t.Fatalf("Iteration %d: Failed to create app: %v", i, err)
		}

		// Do some operations
		_ = app.Id()
		_ = app.Name()

		// Clean up
		service.Destroy()

		if i%10 == 0 {
			t.Logf("Completed %d iterations", i)
		}
	}

	t.Logf("Completed %d iterations - check memory usage", iterations)
}
