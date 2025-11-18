// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"fmt"
	"log"
	"time"

	slim "github.com/agntcy/slim/bindings/generated/slimbindings"
)

func main() {
	fmt.Println("🚀 SLIM Go Bindings Example")
	fmt.Println("============================")

	// Initialize crypto provider (required before any operations)
	slim.InitializeCrypto()
	fmt.Println("✅ Crypto initialized")

	// Get version
	version := slim.GetVersion()
	fmt.Printf("📦 SLIM Bindings Version: %s\n\n", version)

	// Create a service
	service, err := slim.NewService()
	if err != nil {
		log.Fatalf("❌ Failed to create service: %v", err)
	}
	fmt.Println("✅ Service created")

	// Create an app with shared secret authentication
	appName := slim.Name{
		Components: []string{"org", "myapp", "v1"},
		Id:         nil,
	}

	// Note: Shared secret must be at least 32 bytes
	sharedSecret := "my-shared-secret-value-must-be-at-least-32-bytes-long!"

	app, err := service.CreateApp(appName, sharedSecret)
	if err != nil {
		log.Fatalf("❌ Failed to create app: %v", err)
	}

	fmt.Printf("✅ App created with ID: %d\n", app.Id())
	fmt.Printf("   Name: %v\n\n", app.Name())

	// Create a session configuration
	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypePointToPoint,
		EnableMls:   false,
	}

	destination := slim.Name{
		Components: []string{"org", "receiver", "v1"},
		Id:         nil,
	}

	fmt.Println("📡 Creating session to destination...")
	session, err := app.CreateSession(sessionConfig, destination)
	if err != nil {
		log.Fatalf("❌ Failed to create session: %v", err)
	}
	fmt.Println("✅ Session created")
	
	// Ensure session cleanup when done
	defer func() {
		fmt.Println("\n🗑️  Cleaning up session...")
		if err := app.DeleteSession(session); err != nil {
			fmt.Printf("⚠️  Failed to delete session: %v\n", err)
		} else {
			fmt.Println("✅ Session deleted")
		}
	}()

	// Publish a message
	message := []byte("Hello from Go! 👋")
	payloadType := "text/plain"

	fmt.Printf("\n📤 Publishing message: %s\n", string(message))
	err = session.Publish(
		destination,
		1,            // fanout
		message,      // payload
		nil,          // connection_id (optional)
		&payloadType, // payload_type (optional)
		nil,          // metadata (optional)
	)
	if err != nil {
		// This might fail if no receiver is connected, which is expected
		fmt.Printf("⚠️  Publish returned error (expected without receiver): %v\n", err)
	} else {
		fmt.Println("✅ Message published successfully!")
	}

	// Try to receive a message with timeout
	fmt.Println("\n📥 Waiting for message (with 2 second timeout)...")
	timeoutMs := uint32(2000)
	msg, err := session.GetMessage(&timeoutMs)
	if err != nil {
		// Timeout is expected in this simple example since we're not actually
		// connected to another service
		fmt.Printf("⏱️  Timeout or error (expected in this example): %v\n", err)
	} else {
		fmt.Printf("✅ Received message!\n")
		fmt.Printf("   Source: %v\n", msg.Context.SourceName)
		fmt.Printf("   Payload: %s\n", string(msg.Payload))
		fmt.Printf("   Type: %s\n", msg.Context.PayloadType)
	}

	// Demonstrate subscription
	fmt.Println("\n🔔 Managing subscriptions...")
	subscriptionName := slim.Name{
		Components: []string{"org", "events", "notifications"},
		Id:         nil,
	}

	err = app.Subscribe(subscriptionName, nil)
	if err != nil {
		fmt.Printf("⚠️  Subscribe failed (expected without real network): %v\n", err)
	} else {
		fmt.Println("✅ Subscribed to notifications")
	}

	err = app.Unsubscribe(subscriptionName, nil)
	if err != nil {
		fmt.Printf("⚠️  Unsubscribe failed: %v\n", err)
	} else {
		fmt.Println("✅ Unsubscribed from notifications")
	}

	// Demonstrate listening for incoming sessions (with short timeout)
	fmt.Println("\n👂 Listening for incoming sessions (1 second timeout)...")
	listenTimeout := uint32(1000)
	incomingSession, err := app.ListenForSession(&listenTimeout)
	if err != nil {
		fmt.Printf("⏱️  Timeout or error (expected): %v\n", err)
	} else {
		fmt.Printf("✅ Received incoming session: %v\n", incomingSession)
	}

	// Demonstrate metadata in messages
	fmt.Println("\n🏷️  Publishing message with metadata...")
	metadata := map[string]string{
		"timestamp": time.Now().Format(time.RFC3339),
		"sender":    "go-example",
		"priority":  "high",
	}

	messageWithMetadata := []byte("Message with metadata 📦")
	err = session.Publish(
		destination,
		1,
		messageWithMetadata,
		nil,
		&payloadType,
		&metadata,
	)
	if err != nil {
		log.Fatalf("❌ Failed to publish message with metadata: %v", err)
	}
	fmt.Println("✅ Message with metadata published!")

	fmt.Println("\n🎉 Example completed successfully!")
	fmt.Println("\n💡 Note: Some operations may timeout or fail in this example")
	fmt.Println("   because we're not connected to a real SLIM network.")
	fmt.Println("   In a real application, you would have multiple services")
	fmt.Println("   communicating with each other.")
}
