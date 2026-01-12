// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"fmt"
	"log"

	slim "github.com/agntcy/slim/bindings/generated/slim_bindings"
)

func main() {
	fmt.Println("🚀 SLIM Go Bindings Example (Proc Macro Approach)")
	fmt.Println("==================================================")

	// Initialize crypto provider (required before any operations)
	slim.InitializeCryptoProvider()
	fmt.Println("✅ Crypto initialized")

	// Get version
	version := slim.GetVersion()
	fmt.Printf("📦 SLIM Bindings Version: %s\n\n", version)

	// Create an app with shared secret authentication
	appName := slim.NewName("org", "myapp", "v1", nil)

	// Note: Shared secret must be at least 32 bytes
	sharedSecret := "my-shared-secret-value-must-be-at-least-32-bytes-long!"

	// create shared secret provider and verifier
	identityProvider := slim.IdentityProviderConfigSharedSecret{
		Data: sharedSecret,
		Id:   appName.AsString(),
	}

	identityVerifier := slim.IdentityVerifierConfigSharedSecret{
		Data: sharedSecret,
		Id:   appName.AsString(),
	}

	app, err := slim.NewBindingsAdapter(
		appName,
		&identityProvider,
		&identityVerifier,
	)
	if err != nil {
		log.Fatalf("❌ Failed to create app: %v", err)
	}

	fmt.Printf("✅ App created with ID: %d\n", app.Id())
	appNameResult := app.Name()
	fmt.Printf("   Name components: %v\n\n", appNameResult.Components())

	// Create a session configuration
	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypePointToPoint,
		EnableMls:   false,
	}

	destination := slim.NewName("org", "receiver", "v1", nil)

	fmt.Println("📡 Creating session to destination...")
	session, err := app.CreateSessionAndWait(sessionConfig, destination)
	if err != nil {
		log.Fatalf("❌ Failed to create session: %v", err)
	}
	fmt.Println("✅ Session created")

	// Ensure session cleanup when done
	defer func() {
		fmt.Println("\n🗑️  Cleaning up session...")
		if err := app.DeleteSessionAndWait(session); err != nil {
			fmt.Printf("⚠️  Failed to delete session: %v\n", err)
		} else {
			fmt.Println("✅ Session deleted")
		}
	}()

	// Publish a message using simplified API
	message := []byte("Hello from Go! 👋")

	fmt.Println("\n📤 Publishing message...")
	err = session.PublishAndWait(message, nil, nil)
	if err != nil {
		// This might fail without a real SLIM network - that's expected
		fmt.Printf("⚠️  Publish failed (expected without network): %v\n", err)
	} else {
		fmt.Println("✅ Message published successfully")
	}

	// Test subscription
	subscriptionName := slim.NewName("org", "myapp", "events", nil)

	fmt.Println("\n📥 Testing subscription...")
	err = app.Subscribe(subscriptionName, nil)
	if err != nil {
		fmt.Printf("⚠️  Subscribe failed (expected without network): %v\n", err)
	} else {
		fmt.Println("✅ Subscribed successfully")

		// Unsubscribe
		err = app.Unsubscribe(subscriptionName, nil)
		if err != nil {
			fmt.Printf("⚠️  Unsubscribe failed: %v\n", err)
		} else {
			fmt.Println("✅ Unsubscribed successfully")
		}
	}

	// Test invite (will fail for non-multicast session)
	inviteeName := slim.NewName("org", "guest", "v1", nil)

	fmt.Println("\n👥 Testing session invite...")
	err = session.InviteAndWait(inviteeName)
	if err != nil {
		fmt.Printf("⚠️  Invite failed (expected for point-to-point session): %v\n", err)
	} else {
		fmt.Println("✅ Invite sent successfully")
	}

	fmt.Println("\n✨ Example completed successfully!")
	fmt.Println("\n📝 Note: Some operations may fail without a running SLIM network,")
	fmt.Println("   but the bindings are working correctly if you see this message.")
}
