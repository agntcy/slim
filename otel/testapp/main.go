// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"context"
	"flag"
	"os"
	"os/signal"
	"runtime"
	"strings"
	"syscall"
	"time"

	slim "github.com/agntcy/slim/bindings/generated/slim_bindings"
	common "github.com/agntcy/slim/otel"
	"go.uber.org/zap"
)

func main() {
	// Initialize zap logger
	logger, err := zap.NewProduction()
	if err != nil {
		logger.Fatal("Failed to initialize zap logger", zap.Error(err))
	}
	defer logger.Sync()

	// Parse command-line flags
	appNameStr := flag.String("app-name", "agntcy/otel/receiver-app", "Application name in the form org/ns/service")
	serverAddr := flag.String("server", "http://localhost:46357", "SLIM server address")
	sharedSecret := flag.String("secret", "a-very-long-shared-secret-0123456789-abcdefg", "Shared secret for authentication")
	exporterNameStr := flag.String("exporter-name", "", "Optional: exporter application name to invite to sessions")
	mlsEnabled := flag.Bool("mls-enabled", false, "Whether to use MLS")
	flag.Parse()

	// Create and connect app
	app, connId, err := common.CreateAndConnectApp(*appNameStr, *serverAddr, *sharedSecret)
	if err != nil {
		logger.Fatal("Failed to create/connect app", zap.Error(err))
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
		logger.Info("Shutdown signal received")
		cancel()
	}()

	if *exporterNameStr == "" {
		go waitForSessionsAndMessages(app, ctx, logger)
	} else {
		initiateSessions(ctx, app, exporterNameStr, *mlsEnabled, connId, logger)
	}

	// Wait for shutdown signal
	<-ctx.Done()
	logger.Info("Shutting down...")
}

// initiateSessions creates and manages outgoing telemetry sessions
func initiateSessions(ctx context.Context, app *slim.BindingsAdapter, exporterNameStr *string, mlsEnabled bool, connId uint64, logger *zap.Logger) {
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
		logger.Fatal("Invalid application name", zap.Error(err))
	}

	logger.Info("Create session and invite exporter", zap.String("exporter", *exporterNameStr))

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
		logger.Fatal("Failed to create traces session", zap.Error(err))
	}
	sessionTraces.Invite(exporterName)
	time.Sleep(500 * time.Millisecond)
	go handleSession(ctx, app, sessionTraces, common.SignalTraces, logger)

	// create metrics session
	sessionMetrics, err := app.CreateSession(config, metricsChannel)
	if err != nil {
		logger.Fatal("Failed to create metrics session", zap.Error(err))
	}
	sessionMetrics.Invite(exporterName)
	time.Sleep(500 * time.Millisecond)
	go handleSession(ctx, app, sessionMetrics, common.SignalMetrics, logger)
	// create logs session
	sessionLogs, err := app.CreateSession(config, logsChannel)
	if err != nil {
		logger.Fatal("Failed to create logs session", zap.Error(err))
	}
	sessionLogs.Invite(exporterName)
	time.Sleep(500 * time.Millisecond)
	go handleSession(ctx, app, sessionLogs, common.SignalLogs, logger)

	logger.Info("Sessions created. Press Ctrl+C to stop")
}

// waitForSessionsAndMessages listens for incoming sessions and handles messages from each session concurrently
func waitForSessionsAndMessages(app *slim.BindingsAdapter, ctx context.Context, logger *zap.Logger) {
	// Lock this goroutine to an OS thread to prevent FFI calls from blocking other goroutines
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()

	logger.Info("Waiting for incoming sessions...")
	logger.Info("Press Ctrl+C to stop")

	for {
		select {
		case <-ctx.Done():
			logger.Info("Shutting down...")
			return

		default:
			// Wait for new session with timeout
			timeout := uint32(1000) // 1 sec
			session, err := app.ListenForSession(&timeout)
			if err != nil {
				// Timeout is normal, just continue
				continue
			}

			dst, err := session.Destination()
			if err != nil {
				logger.Error("error getting destination from new received session", zap.Error(err))
				continue
			}

			if len(dst.Components) < 3 {
				logger.Error("session destination has insufficient components")
				continue
			}

			// Extract signal type from dst.Components[2] suffix
			telemetryType := dst.Components[2]
			signalType, err := common.ExtractSignalType(telemetryType)
			if err != nil {
				logger.Error("error extracting signal type", zap.Error(err))
				continue
			}

			logger.Info("New session established", zap.String("telemetryType", string(signalType)))
			// Handle the session in a goroutine
			go handleSession(ctx, app, session, signalType, logger)
		}
	}
}

// handleSession processes messages from a single session
func handleSession(ctx context.Context, app *slim.BindingsAdapter, session *slim.BindingsSessionContext, signalType common.SignalType, logger *zap.Logger) {
	// Lock this goroutine to an OS thread to prevent FFI calls from blocking other goroutines
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()

	sessionNum, err := session.SessionId()
	if err != nil {
		logger.Error("error getting session ID", zap.Error(err))
		return
	}

	defer func() {
		if err := app.DeleteSession(session); err != nil {
			logger.Warn("failed to delete session", zap.Uint32("sessionId", sessionNum), zap.String("signalType", string(signalType)), zap.Error(err))
		}
		logger.Info("Session closed", zap.Uint32("sessionId", sessionNum), zap.String("signalType", string(signalType)))
	}()

	messageCount := 0

	for {
		select {
		case <-ctx.Done():
			logger.Info("Shutting down session",
				zap.Uint32("sessionId", sessionNum),
				zap.String("signalType", string(signalType)),
				zap.Int("totalMessages", messageCount))
			return
		default:
			// Wait for message with timeout
			timeout := uint32(10000) // 1 sec
			msg, err := session.GetMessage(&timeout)
			if err != nil {
				errMsg := err.Error()
				if contains(errMsg, "session closed") {
					logger.Info("Session closed by peer", zap.Uint32("sessionId", sessionNum), zap.Error(err))
					return
				} else if contains(errMsg, "receive timeout waiting for message") {
					// Normal timeout, continue
					continue
				} else {
					logger.Error("Error getting message", zap.Uint32("sessionId", sessionNum), zap.Error(err))
					continue
				}
			}

			messageCount++
			logger.Info("Received message",
				zap.Uint32("sessionId", sessionNum),
				zap.String("signalType", string(signalType)),
				zap.Int("messageNumber", messageCount),
				zap.Int("sizeBytes", len(msg.Payload)))
		}
	}
}

// contains checks if a string contains a substring
func contains(s, substr string) bool {
	return strings.Contains(s, substr)
}
