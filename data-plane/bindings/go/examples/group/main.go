// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"bufio"
	"flag"
	"fmt"
	"log"
	"os"
	"strings"
	"sync"
	"time"

	slim "github.com/agntcy/slim-bindings-go"
	"github.com/agntcy/slim/bindings/go/examples/common"
)

// ANSI color codes for terminal output
const (
	colorReset  = "\033[0m"
	colorCyan   = "\033[96m"
	colorYellow = "\033[93m"
	colorGreen  = "\033[92m"
)

func main() {
	// Command-line flags
	local := flag.String("local", "", "Local ID (org/namespace/app) - required")
	remote := flag.String("remote", "", "Remote ID (org/namespace/channel) - for group topic")
	server := flag.String("server", common.DefaultServerEndpoint, "SLIM server endpoint")
	sharedSecret := flag.String("shared-secret", common.DefaultSharedSecret, "Shared secret (min 32 chars)")
	enableMLS := flag.Bool("enable-mls", false, "Enable MLS encryption")
	invites := flag.String("invites", "", "Comma-separated list of participant IDs to invite (moderator mode)")

	flag.Parse()

	if *local == "" {
		log.Fatal("--local is required")
	}

	// Parse invites
	var inviteList []string
	if *invites != "" {
		inviteList = strings.Split(*invites, ",")
		for i, inv := range inviteList {
			inviteList[i] = strings.TrimSpace(inv)
		}
	}

	// Determine mode
	isModerator := *remote != "" && len(inviteList) > 0

	// Create and connect app
	app, connID, err := common.CreateAndConnectApp(*local, *server, *sharedSecret)
	if err != nil {
		log.Fatalf("Failed to create/connect app: %v", err)
	}
	defer app.Destroy()

	instance := app.Id()
	fmt.Printf("%s[%d]%s ✅ Created app\n", colorCyan, instance, colorReset)
	fmt.Printf("%s[%d]%s 🔌 Connected to %s (conn ID: %d)\n", colorCyan, instance, colorReset, *server, connID)

	// Run in appropriate mode
	if isModerator {
		runModerator(app, connID, *remote, inviteList, *enableMLS, instance)
	} else {
		runParticipant(app, instance)
	}
}

func runModerator(app *slim.App, connID uint64, remote string, invites []string, enableMLS bool, instance uint64) {
	// Parse remote channel name
	channelName, err := common.SplitID(remote)
	if err != nil {
		log.Fatalf("Failed to parse remote channel: %v", err)
	}

	fmt.Printf("%s[%d]%s 📡 Creating group session as moderator for channel: %s\n", colorCyan, instance, colorReset, remote)

	// Create multicast session
	interval := time.Second * 5
	config := slim.SessionConfig{
		SessionType: slim.SessionTypeGroup,
		EnableMls:   enableMLS,
		MaxRetries:  &[]uint32{5}[0], // 5 retries
		Interval:    &interval,
		Metadata:    make(map[string]string),
	}

	session, err := app.CreateSessionAndWaitAsync(config, channelName)
	if err != nil {
		log.Fatalf("Failed to create session: %v", err)
	}
	defer app.DeleteSessionAndWaitAsync(session)

	// Give session a moment to establish
	time.Sleep(100 * time.Millisecond)
	fmt.Printf("%s[%d]%s ✅ Group session created\n", colorCyan, instance, colorReset)

	// Invite each participant
	for _, inviteID := range invites {
		inviteName, err := common.SplitID(inviteID)
		if err != nil {
			log.Printf("Failed to parse invite ID %s: %v", inviteID, err)
			continue
		}

		// Set route for invitee
		if err := app.SetRouteAsync(inviteName, connID); err != nil {
			log.Printf("Failed to set route for %s: %v", inviteID, err)
			continue
		}

		// Send invite
		if err := session.InviteAndWaitAsync(inviteName); err != nil {
			log.Printf("Failed to invite %s: %v", inviteID, err)
			continue
		}

		fmt.Printf("%s[%d]%s 👥 Invited %s to the group\n", colorCyan, instance, colorReset, inviteID)
	}

	// Run message loops
	runMessageLoops(app, session, channelName, instance)
}

func runParticipant(app *slim.App, instance uint64) {
	fmt.Printf("%s[%d]%s 👂 Waiting for incoming group session invitation...\n", colorCyan, instance, colorReset)

	// Wait for incoming session
	timeout := time.Second * 60 // 60 seconds timeout
	session, err := app.ListenForSessionAsync(&timeout)
	if err != nil {
		log.Fatalf("Failed to receive session: %v", err)
	}
	defer app.DeleteSessionAndWaitAsync(session)

	channelName, err := session.Destination()
	if err != nil {
		log.Fatalf("Failed to get session destination: %v", err)
	}

	fmt.Printf("%s[%d]%s 🎉 Joined group session for channel: %v\n", colorCyan, instance, colorReset, channelName.Components())

	// Run message loops
	runMessageLoops(app, session, channelName, instance)
}

func runMessageLoops(app *slim.App, session *slim.Session, channelName *slim.Name, instance uint64) {
	var wg sync.WaitGroup
	stopChan := make(chan struct{})

	sourceName, err := session.Source()
	if err != nil {
		log.Fatalf("Failed to get session source: %v", err)
	}

	// Start receive goroutine
	wg.Add(1)
	go func() {
		defer wg.Done()
		receiveLoop(session, sourceName, instance, stopChan)
	}()

	// Start keyboard input goroutine
	wg.Add(1)
	go func() {
		defer wg.Done()
		keyboardLoop(session, sourceName, channelName, instance, stopChan)
	}()

	// Wait for both goroutines
	wg.Wait()
}

func receiveLoop(session *slim.Session, sourceName *slim.Name, instance uint64, stopChan chan struct{}) {
	for {
		select {
		case <-stopChan:
			return
		default:
			timeout := time.Second * 1
			msg, err := session.GetMessageAsync(&timeout)
			if err != nil {
				// Check if it's just a timeout
				if strings.Contains(err.Error(), "timed out") || strings.Contains(err.Error(), "timeout") {
					continue
				}
				fmt.Printf("%s[%d]%s 🔚 Session ended: %v\n", colorCyan, instance, colorReset, err)
				close(stopChan)
				return
			}

			// Display received message
			fmt.Printf("\n%s%s > %s%s\n", colorYellow, msg.Context.SourceName.Components(), string(msg.Payload), colorReset)

			// Auto-reply if this is not already a reply (prevent loops)
			if _, hasPublishTo := msg.Context.Metadata["PUBLISH_TO"]; !hasPublishTo {
				reply := fmt.Sprintf("message received by %v", sourceName.Components())
				if err := session.PublishToAndWaitAsync(msg.Context, []byte(reply), nil, nil); err != nil {
					fmt.Printf("%s[%d]%s ❌ Error sending reply: %v\n", colorCyan, instance, colorReset, err)
				}
			}

			// Re-print prompt
			fmt.Printf("%s%v > %s", colorGreen, sourceName.Components(), colorReset)
		}
	}
}

func keyboardLoop(session *slim.Session, sourceName, channelName *slim.Name, instance uint64, stopChan chan struct{}) {
	reader := bufio.NewReader(os.Stdin)

	fmt.Printf("\n%s[%d]%s Welcome to the group %v!\n", colorCyan, instance, colorReset, channelName.Components())
	listenerNames, err := session.ParticipantsList()
	if err != nil {
		log.Fatalf("Failed to get participants list: %v", err)
	}

	// Display participants
	fmt.Printf("%s[%d]%s 👥 Participants in the group:\n", colorCyan, instance, colorReset)
	for _, participant := range listenerNames {
		fmt.Printf("%s[%d]%s 👤 Participant: %v\n", colorCyan, instance, colorReset, participant.Components())
	}

	fmt.Printf("%s[%d]%s Type a message and press Enter to send, or 'exit'/'quit' to leave.\n\n", colorCyan, instance, colorReset)
	fmt.Printf("%s%v > %s", colorGreen, sourceName.Components(), colorReset)

	for {
		select {
		case <-stopChan:
			return
		default:
			// Read input (this blocks, but that's okay for user input)
			input, err := reader.ReadString('\n')
			if err != nil {
				fmt.Printf("\n%s[%d]%s ❌ Error reading input: %v\n", colorCyan, instance, colorReset, err)
				close(stopChan)
				return
			}

			input = strings.TrimSpace(input)
			if input == "" {
				fmt.Printf("%s%v > %s", colorGreen, sourceName.Components(), colorReset)
				continue
			}

			// Check for exit commands
			if strings.ToLower(input) == "exit" || strings.ToLower(input) == "quit" {
				fmt.Printf("\n%s[%d]%s 👋 Leaving group...\n", colorCyan, instance, colorReset)
				close(stopChan)
				return
			}

			// Publish message to group
			if err := session.PublishAndWaitAsync([]byte(input), nil, nil); err != nil {
				fmt.Printf("%s[%d]%s ❌ Error sending message: %v\n", colorCyan, instance, colorReset, err)
			} else {
				fmt.Printf("%s[%d]%s 📤 Sent to group\n", colorCyan, instance, colorReset)
			}

			fmt.Printf("%s%v > %s", colorGreen, sourceName.Components(), colorReset)
		}
	}
}
