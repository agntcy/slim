// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"context"
	"flag"
	"fmt"
	"log"
	"os"
	"os/signal"
	"syscall"
	"time"

	slim "github.com/agntcy/slim/bindings/generated/slim_bindings"
	common "github.com/agntcy/slim/otel"
)

func main() {
	// Parse command-line flags
	appNameStr := flag.String("app-name", "agntcy/otel/receiver-app", "Application name in the form org/ns/service")
	expoterNameStr := flag.String("exporter-name", "agntcy/otel/exporter", "Exporter application name")
	serverAddr := flag.String("server", "http://localhost:46357", "SLIM server address")
	sharedSecret := flag.String("secret", "a-very-log-shared-secret-0123456789-abcdefg", "Shared secret for authentication")
	initiator := flag.Bool("initiator", true, "Whether this app initiates sessions")
	mlsEnabled := flag.Bool("mls-enabled", false, "Whether to use MLS")
	flag.Parse()

	// Create and connect app
	app, connId, err := common.CreateAndConnectApp(*appNameStr, *serverAddr, *sharedSecret)
	if err != nil {
		log.Fatalf("Failed to create/connect app: %v", err)
	}
	defer app.Destroy()

	// Set up context with cancellation for graceful shutdown
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	// Set up signal handling for Ctrl+C
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, os.Interrupt, syscall.SIGTERM)

	go func() {
		<-sigChan
		fmt.Println("\n\nShutdown signal received...")
		cancel()
	}()

	if !*initiator {
		waitForSessionsAndMessages(app, ctx)
	} else {
		initiateSessions(ctx, app, expoterNameStr, *mlsEnabled, connId)
	}
}

// initiateSessions creates and manages outgoing telemetry sessions
func initiateSessions(ctx context.Context, app *slim.BindingsAdapter, exporterNameStr *string, mlsEnabled bool, connId uint64) {
	// create names telemetry channel
	tracesChannel := slim.Name{
		Components: []string{"agntcy", "otel", "telemetry-traces"},
		Id:         nil,
	}
	metricsChannel := slim.Name{
		Components: []string{"agntcy", "otel", "telemetry-metrics"},
		Id:         nil,
	}
	logsChannel := slim.Name{
		Components: []string{"agntcy", "otel", "telemetry-logs"},
		Id:         nil,
	}
	exporterName, err := common.SplitID(*exporterNameStr)
	if err != nil {
		log.Fatalf("Invalid application name: %v", err)
	}

	maxRetries := uint32(10)
	intervalMs := uint64(1000)
	config := slim.SessionConfig{
		SessionType: slim.SessionTypeGroup,
		EnableMls:   mlsEnabled,
		MaxRetries:  &maxRetries,
		IntervalMs:  &intervalMs,
		Initiator:   true,
	}

	app.SetRoute(exporterName, connId)
	// create traces session
	sessionTraces, err := app.CreateSession(config, tracesChannel)
	if err != nil {
		log.Fatalf("Failed to create traces session: %v", err)
	}
	sessionTraces.Invite(exporterName)
	time.Sleep(500 * time.Millisecond)
	go handleSession(app, sessionTraces, 1, ctx)

	// create metrics session
	sessionMetrics, err := app.CreateSession(config, metricsChannel)
	if err != nil {
		log.Fatalf("Failed to create metrics session: %v", err)
	}
	sessionMetrics.Invite(exporterName)
	time.Sleep(500 * time.Millisecond)
	go handleSession(app, sessionMetrics, 2, ctx)

	// create logs session
	sessionLogs, err := app.CreateSession(config, logsChannel)
	if err != nil {
		log.Fatalf("Failed to create logs session: %v", err)
	}
	sessionLogs.Invite(exporterName)
	time.Sleep(500 * time.Millisecond)
	go handleSession(app, sessionLogs, 3, ctx)

	fmt.Println("Sessions created. Press Ctrl+C to stop")

	// Wait for shutdown signal
	<-ctx.Done()
	fmt.Println("Shutting down...")
}

// waitForSessionsAndMessages listens for incoming sessions and handles messages from each session concurrently
func waitForSessionsAndMessages(app *slim.BindingsAdapter, ctx context.Context) {
	fmt.Println("Waiting for incoming sessions...")
	fmt.Println("Press Ctrl+C to stop")

	sessionCount := 0

	for {
		select {
		case <-ctx.Done():
			fmt.Printf("\nHandled %d sessions total\n", sessionCount)
			fmt.Println("Shutting down...")
			return

		default:
			// Wait for new session with timeout
			timeout := uint32(1000) // 1 sec
			session, err := app.ListenForSession(&timeout)
			if err != nil {
				// Timeout is normal, just continue
				continue
			}

			sessionCount++
			dst, _ := session.Destination()
			fmt.Printf("\n[Session %d] New session established! Telemetry Type %s\n", sessionCount, dst.Components[2])

			// Handle the session in a goroutine
			go handleSession(app, session, sessionCount, ctx)
		}
	}
}

// handleSession processes messages from a single session
func handleSession(app *slim.BindingsAdapter, session *slim.BindingsSessionContext, sessionNum int, ctx context.Context) {
	defer func() {
		if err := app.DeleteSession(session); err != nil {
			log.Printf("[Session %d] Warning: failed to delete session: %v", sessionNum, err)
		}
		fmt.Printf("[Session %d] Session closed\n", sessionNum)
	}()

	messageCount := 0

	for {
		select {
		case <-ctx.Done():
			fmt.Printf("[Session %d] Shutting down after %d messages\n", sessionNum, messageCount)
			return
		default:
		}

		// Wait for message with timeout
		timeout := uint32(1000) // 1 sec
		msg, err := session.GetMessage(&timeout)
		if err != nil {
			// Timeout is normal, just continue
			continue
		}

		messageCount++

		// Print telemetry data received
		fmt.Printf("[Session %d] Message %d: received %d bytes of telemetry data\n",
			sessionNum, messageCount, len(msg.Payload))

		// TODO: Parse and process OTLP data here
		// The msg.Payload should contain OTLP-formatted traces, metrics, or logs
		// You could unmarshal this using the OTLP protobuf definitions

		// For now, just print a preview of the data
		if len(msg.Payload) > 0 {
			preview := msg.Payload
			if len(preview) > 100 {
				preview = preview[:100]
			}
			fmt.Printf("[Session %d]    Preview: %s...\n\n", sessionNum, string(preview))
		}
	}
}
