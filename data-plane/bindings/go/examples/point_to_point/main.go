// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"flag"
	"fmt"
	"log"
	"time"

	slim "github.com/agntcy/slim-bindings-go"
	"github.com/agntcy/slim/bindings/go/examples/common"
)

func main() {
	// Command-line flags
	local := flag.String("local", "", "Local ID (org/namespace/app) - required")
	remote := flag.String("remote", "", "Remote ID (org/namespace/app)")
	server := flag.String("server", common.DefaultServerEndpoint, "SLIM server endpoint")
	message := flag.String("message", "", "Message to send (sender mode)")
	iterations := flag.Int("iterations", 10, "Number of messages to send")
	sharedSecret := flag.String("shared-secret", common.DefaultSharedSecret, "Shared secret (min 32 chars)")
	enableMLS := flag.Bool("enable-mls", false, "Enable MLS encryption")

	flag.Parse()

	if *local == "" {
		log.Fatal("--local is required")
	}

	// Create and connect app
	app, connID, err := common.CreateAndConnectApp(*local, *server, *sharedSecret)
	if err != nil {
		log.Fatalf("Failed to create/connect app: %v", err)
	}
	defer app.Destroy()

	instance := app.Id()
	fmt.Printf("[%d] ✅ Created app\n", instance)
	fmt.Printf("[%d] 🔌 Connected to %s (conn ID: %d)\n", instance, *server, connID)

	// Run sender or receiver mode
	if *message != "" && *remote != "" {
		runSender(app, connID, *remote, *message, *iterations, *enableMLS, instance)
	} else if *message != "" {
		log.Fatal("--remote required when --message specified")
	} else {
		runReceiver(app, instance)
	}
}

func runSender(app *slim.App, connID uint64, remote, message string, iterations int, enableMLS bool, instance uint64) {
	remoteName, err := slim.NameFromString(remote)
	if err != nil {
		log.Fatalf("Failed to parse remote ID: %v", err)
	}

	// Set route to remote via the server connection
	if err := app.SetRouteAsync(remoteName, connID); err != nil {
		log.Fatalf("Failed to set route: %v", err)
	}
	fmt.Printf("[%d] 📍 Route set to %s via connection %d\n", instance, remote, connID)

	config := slim.SessionConfig{
		SessionType: slim.SessionTypePointToPoint,
		EnableMls:   enableMLS,
	}

	fmt.Printf("[%d] 🔍 Creating session to %s...\n", instance, remote)
	session, err := app.CreateSessionAndWaitAsync(config, remoteName)
	if err != nil {
		log.Fatalf("Failed to create session: %v", err)
	}
	defer app.DeleteSessionAndWaitAsync(session)

	// Give session a moment to establish
	time.Sleep(100 * time.Millisecond)

	fmt.Printf("[%d] 📡 Session created\n", instance)

	for i := 0; i < iterations; i++ {
		if err := session.PublishAndWaitAsync([]byte(message), nil, nil); err != nil {
			fmt.Printf("[%d] ❌ Error sending message %d/%d: %v\n", instance, i+1, iterations, err)
			continue
		}

		fmt.Printf("[%d] 📤 Sent message '%s' - %d/%d\n", instance, message, i+1, iterations)

		// Wait for reply
		timeout := time.Second * 5
		msg, err := session.GetMessageAsync(&timeout)
		if err != nil {
			fmt.Printf("[%d] ⏱️  No reply for message %d/%d: %v\n", instance, i+1, iterations, err)
			continue
		}

		fmt.Printf("[%d] 📥 Received reply '%s' - %d/%d\n", instance, string(msg.Payload), i+1, iterations)
		time.Sleep(1 * time.Second)
	}
}

func runReceiver(app *slim.App, instance uint64) {
	fmt.Printf("[%d] 👂 Waiting for incoming sessions...\n", instance)

	for {
		session, err := app.ListenForSessionAsync(nil)
		if err != nil {
			fmt.Printf("[%d] ⏱️  Timeout waiting for session, retrying...\n", instance)
			continue
		}

		fmt.Printf("[%d] 🎉 New session established!\n", instance)
		go handleSession(app, session, instance)
	}
}

func handleSession(app *slim.App, session *slim.Session, instance uint64) {
	defer func() {
		if err := app.DeleteSessionAndWaitAsync(session); err != nil {
			log.Printf("[%d] ⚠️  Warning: failed to delete session: %v", instance, err)
		}
		fmt.Printf("[%d] 👋 Session closed\n", instance)
	}()

	for {
		timeout := time.Second * 60
		msg, err := session.GetMessageAsync(&timeout)
		if err != nil {
			fmt.Printf("[%d] 🔚 Session ended: %v\n", instance, err)
			break
		}

		text := string(msg.Payload)
		fmt.Printf("[%d] 📨 Received: %s\n", instance, text)

		reply := fmt.Sprintf("%s from %d", text, instance)
		if err := session.PublishToAndWaitAsync(msg.Context, []byte(reply), nil, nil); err != nil {
			log.Printf("[%d] ❌ Error sending reply: %v", instance, err)
			break
		}

		fmt.Printf("[%d] 📤 Replied: %s\n", instance, reply)
	}
}
