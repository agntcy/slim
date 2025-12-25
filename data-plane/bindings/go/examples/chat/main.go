// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Package main implements a console chat application using SLIM.
//
// Usage:
//
//	# Create a new chat room (moderator mode)
//	chat --room myroom --user alice --create
//
//	# Join an existing chat room
//	chat --room myroom --user bob
//
// The moderator creates the room and automatically invites users who send
// join requests. Participants send a join request and wait to be invited.
package main

import (
	"bufio"
	"flag"
	"fmt"
	"log"
	"os"
	"os/signal"
	"strings"
	"sync"
	"syscall"
	"time"

	slim "github.com/agntcy/slim/bindings/generated/slim_bindings"
	"github.com/agntcy/slim/bindings/go/examples/common"
)

// ANSI color codes for terminal output
const (
	colorReset  = "\033[0m"
	colorCyan   = "\033[96m"  // System messages
	colorYellow = "\033[93m"  // Other users' messages
	colorGreen  = "\033[92m"  // Your messages/prompt
	colorBlue   = "\033[94m"  // Timestamps
	colorRed    = "\033[91m"  // Errors
	colorMagenta = "\033[95m" // Join/leave notifications
)

// Message metadata keys
const (
	metaUsername  = "username"
	metaPublishTo = "PUBLISH_TO"
)

func main() {
	// Command-line flags
	room := flag.String("room", "", "Chat room name (required)")
	user := flag.String("user", "", "Your username (required)")
	create := flag.Bool("create", false, "Create a new room as moderator")
	invites := flag.String("invites", "", "Comma-separated usernames to invite (moderator mode)")
	server := flag.String("server", common.DefaultServerEndpoint, "SLIM server endpoint")
	secret := flag.String("secret", common.DefaultSharedSecret, "Shared secret for authentication")
	enableMLS := flag.Bool("mls", false, "Enable MLS encryption")

	flag.Parse()

	// Validate required flags
	if *room == "" {
		log.Fatal("--room is required")
	}
	if *user == "" {
		log.Fatal("--user is required")
	}

	// Build identity: chat/<room>/user/<username>
	localID := fmt.Sprintf("chat/%s/%s", *room, *user)
	channelID := fmt.Sprintf("chat/%s/channel", *room)

	// Parse invites list
	var inviteList []string
	if *invites != "" {
		for _, inv := range strings.Split(*invites, ",") {
			inv = strings.TrimSpace(inv)
			if inv != "" {
				// Convert username to full ID
				inviteList = append(inviteList, fmt.Sprintf("chat/%s/%s", *room, inv))
			}
		}
	}

	// Create and connect app
	app, connID, err := common.CreateAndConnectApp(localID, *server, *secret)
	if err != nil {
		log.Fatalf("Failed to create/connect app: %v", err)
	}
	defer app.Destroy()

	printSystem("Connected to %s", *server)

	// Set route for the channel
	channelName, err := common.SplitID(channelID)
	if err != nil {
		log.Fatalf("Failed to parse channel ID: %v", err)
	}
	if err := app.SetRoute(channelName, connID); err != nil {
		log.Fatalf("Failed to set route: %v", err)
	}

	// Handle Ctrl+C gracefully
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)

	// Run in appropriate mode
	if *create {
		runModerator(app, connID, *room, *user, channelName, inviteList, *enableMLS, sigChan)
	} else {
		runParticipant(app, connID, *room, *user, channelName, sigChan)
	}
}

// runModerator creates a chat room and handles incoming join requests
func runModerator(app *slim.BindingsAdapter, connID uint64, room, username string, channelName slim.Name, invites []string, enableMLS bool, sigChan chan os.Signal) {
	printSystem("Creating chat room '%s' as moderator...", room)

	// Create group session
	config := slim.SessionConfig{
		SessionType: slim.SessionTypeGroup,
		EnableMls:   enableMLS,
		MaxRetries:  ptr(uint32(5)),
		IntervalMs:  ptr(uint64(5000)),
		Initiator:   true,
		Metadata:    make(map[string]string),
	}

	session, err := app.CreateSession(config, channelName)
	if err != nil {
		log.Fatalf("Failed to create session: %v", err)
	}
	defer app.DeleteSession(session)

	// Wait a moment for session to establish
	time.Sleep(100 * time.Millisecond)
	printSystem("Chat room '%s' created!", room)

	// Invite initial participants
	for _, inviteID := range invites {
		inviteName, err := common.SplitID(inviteID)
		if err != nil {
			printError("Failed to parse invite ID %s: %v", inviteID, err)
			continue
		}

		if err := app.SetRoute(inviteName, connID); err != nil {
			printError("Failed to set route for %s: %v", inviteID, err)
			continue
		}

		if err := session.Invite(inviteName); err != nil {
			printError("Failed to invite %s: %v", inviteID, err)
			continue
		}

		// Extract username from invite ID
		parts := strings.Split(inviteID, "/")
		printJoin("%s has been invited", parts[len(parts)-1])
	}

	// Run chat loop
	runChatLoop(app, session, connID, room, username, true, sigChan)
}

// runParticipant waits for an invitation from the moderator
func runParticipant(app *slim.BindingsAdapter, connID uint64, room, username string, channelName slim.Name, sigChan chan os.Signal) {
	printSystem("Waiting for invitation to chat room '%s'...", room)
	printSystem("(Ask the moderator to invite you with: --invites %s)", username)

	// Wait for invitation to the group
	timeout := uint32(120000) // 120 seconds
	session, err := app.ListenForSession(&timeout)
	if err != nil {
		log.Fatalf("Failed to receive invitation (timeout or room not active): %v", err)
	}
	defer app.DeleteSession(session)

	printSystem("Joined chat room '%s'!", room)

	// Run chat loop
	runChatLoop(app, session, connID, room, username, false, sigChan)
}

// runChatLoop handles sending and receiving messages
func runChatLoop(app *slim.BindingsAdapter, session *slim.BindingsSessionContext, connID uint64, room, username string, isModerator bool, sigChan chan os.Signal) {
	var wg sync.WaitGroup
	stopChan := make(chan struct{})

	// Print welcome message
	fmt.Println()
	printSystem("Welcome to chat room '%s'!", room)
	printSystem("Type a message and press Enter to send, or 'exit' to leave.")
	fmt.Println()
	printPrompt(username)

	// Handle Ctrl+C
	go func() {
		<-sigChan
		fmt.Println()
		printSystem("Shutting down...")
		close(stopChan)
	}()

	// Start receive goroutine
	wg.Add(1)
	go func() {
		defer wg.Done()
		receiveLoop(session, username, stopChan)
	}()

	// Start keyboard input goroutine
	wg.Add(1)
	go func() {
		defer wg.Done()
		keyboardLoop(session, username, stopChan)
	}()

	wg.Wait()
	printSystem("Goodbye!")
}

// receiveLoop handles incoming messages
func receiveLoop(session *slim.BindingsSessionContext, username string, stopChan chan struct{}) {
	for {
		select {
		case <-stopChan:
			return
		default:
			timeout := uint32(1000) // 1 second timeout for checking stopChan
			msg, err := session.GetMessage(&timeout)
			if err != nil {
				if strings.Contains(err.Error(), "timed out") || strings.Contains(err.Error(), "timeout") {
					continue
				}
				printError("Session ended: %v", err)
				close(stopChan)
				return
			}

			// Skip if this is a reply marker message
			if _, hasPublishTo := msg.Context.Metadata[metaPublishTo]; hasPublishTo {
				continue
			}

			// Display regular chat message
			displayMessage(msg, username)
			printPrompt(username)
		}
	}
}

// keyboardLoop reads user input and sends messages
func keyboardLoop(session *slim.BindingsSessionContext, username string, stopChan chan struct{}) {
	reader := bufio.NewReader(os.Stdin)

	for {
		select {
		case <-stopChan:
			return
		default:
			input, err := reader.ReadString('\n')
			if err != nil {
				printError("Error reading input: %v", err)
				close(stopChan)
				return
			}

			input = strings.TrimSpace(input)
			if input == "" {
				printPrompt(username)
				continue
			}

			// Check for exit commands
			lower := strings.ToLower(input)
			if lower == "exit" || lower == "quit" || lower == "/exit" || lower == "/quit" {
				close(stopChan)
				return
			}

			// Send message with username metadata
			metadata := map[string]string{
				metaUsername: username,
			}
			if err := session.Publish([]byte(input), nil, &metadata); err != nil {
				printError("Failed to send: %v", err)
			}

			printPrompt(username)
		}
	}
}

// displayMessage formats and prints a received message
func displayMessage(msg slim.ReceivedMessage, myUsername string) {
	// Get sender username from metadata or source name
	sender := msg.Context.Metadata[metaUsername]
	if sender == "" {
		parts := msg.Context.SourceName.Components
		if len(parts) > 0 {
			sender = parts[len(parts)-1]
		} else {
			sender = "unknown"
		}
	}

	// Skip our own messages
	if sender == myUsername {
		return
	}

	// Format timestamp
	timestamp := time.Now().Format("15:04:05")
	text := string(msg.Payload)

	// Print with colors
	fmt.Printf("\r%s[%s]%s %s%s:%s %s\n",
		colorBlue, timestamp, colorReset,
		colorYellow, sender, colorReset,
		text)
}

// Helper functions for colored output

func printSystem(format string, args ...interface{}) {
	msg := fmt.Sprintf(format, args...)
	fmt.Printf("%s[system]%s %s\n", colorCyan, colorReset, msg)
}

func printError(format string, args ...interface{}) {
	msg := fmt.Sprintf(format, args...)
	fmt.Printf("%s[error]%s %s\n", colorRed, colorReset, msg)
}

func printJoin(format string, args ...interface{}) {
	msg := fmt.Sprintf(format, args...)
	fmt.Printf("%s*** %s ***%s\n", colorMagenta, msg, colorReset)
}

func printPrompt(username string) {
	fmt.Printf("%s%s >%s ", colorGreen, username, colorReset)
}

// ptr is a helper to create pointers to values
func ptr[T any](v T) *T {
	return &v
}
