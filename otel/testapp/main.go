// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"strings"
	"sync"
	"syscall"
	"time"

	"go.uber.org/zap"

	slim "github.com/agntcy/slim-bindings-go"
	common "github.com/agntcy/slim/otel"
)

func main() {
	// Initialize zap logger
	logger, err := zap.NewProduction()
	if err != nil {
		logger.Fatal("Failed to initialize zap logger", zap.Error(err))
	}
	defer func() { _ = logger.Sync() }()

	// Parse command-line flags
	appNameStr := flag.String("app-name", "agntcy/otel/receiver-app",
		"Application name in the form org/ns/service")
	serverAddr := flag.String("server", "http://localhost:46357", "SLIM server address")
	sharedSecret := flag.String("secret", "a-very-long-shared-secret-0123456789-abcdefg",
		"Shared secret for authentication")
	exporterNameStr := flag.String("exporter-name", "",
		"Optional: exporter application name to invite to sessions")
	channelNameStr := flag.String("channel-name", "",
		"Optional: channel name to receive telemetry, required if the exporter name is provided")
	mlsEnabled := flag.Bool("mls-enabled", false, "Whether to use MLS")
	flag.Parse()

	// Create and connect app
	app, connID, err := common.CreateAndConnectApp(*appNameStr, *serverAddr, *sharedSecret)
	if err != nil {
		logger.Fatal("Failed to create/connect app", zap.Error(err))
	}
	defer app.Destroy()

	// Set up context with cancellation for graceful shutdown
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	// WaitGroup to track active sessions
	var wg sync.WaitGroup

	// Set up signal handling for Ctrl+C
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, os.Interrupt, syscall.SIGTERM)

	go func() {
		<-sigChan
		logger.Info("Shutdown signal received")
		cancel()
	}()

	if *exporterNameStr == "" {
		go waitForSessionsAndMessages(ctx, &wg, logger, app)
	} else {
		if *channelNameStr == "" {
			logger.Fatal("channel-name must be provided when inviter-name is set")
		}
		initiateSessions(ctx, &wg, logger, app, exporterNameStr, channelNameStr, connID, *mlsEnabled)
	}

	// Wait for shutdown signal
	<-ctx.Done()
	logger.Info("Shutting down...")

	// Wait for all sessions to close with timeout
	done := make(chan struct{})
	go func() {
		wg.Wait()
		close(done)
	}()

	select {
	case <-done:
		logger.Info("All sessions closed gracefully")
	case <-time.After(15 * time.Second):
		logger.Warn("Shutdown timeout reached, forcing exit")
	}
}

// initiateSessions creates and manages outgoing telemetry sessions
func initiateSessions(
	ctx context.Context,
	wg *sync.WaitGroup,
	logger *zap.Logger,
	app *slim.BindingsAdapter,
	exporterNameStr *string,
	channelNameStr *string,
	connID uint64,
	mlsEnabled bool,
) {
	// create names telemetry channel
	channel := fmt.Sprintf("%s-%s", *channelNameStr, "traces")
	tracesChannel, err := common.SplitID(channel)
	if err != nil {
		logger.Fatal("Invalid traces channel name", zap.Error(err))
	}
	channel = fmt.Sprintf("%s-%s", *channelNameStr, "metrics")
	metricsChannel, err := common.SplitID(channel)
	if err != nil {
		logger.Fatal("Invalid metrics channel name", zap.Error(err))
	}
	channel = fmt.Sprintf("%s-%s", *channelNameStr, "logs")
	logsChannel, err := common.SplitID(channel)
	if err != nil {
		logger.Fatal("Invalid logs channel name", zap.Error(err))
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

	err = app.SetRoute(exporterName, connID)
	if err != nil {
		logger.Fatal("Failed to set route", zap.Error(err))
	}
	// create traces session
	sessionTraces, err := app.CreateSession(config, tracesChannel)
	if err != nil {
		logger.Fatal("Failed to create traces session", zap.Error(err))
	}
	err = sessionTraces.Invite(exporterName)
	if err != nil {
		logger.Fatal("Failed to invite exporter to traces session", zap.Error(err))
	}
	time.Sleep(500 * time.Millisecond)
	wg.Add(1)
	go handleSession(ctx, wg, logger, app, sessionTraces, common.SignalTraces)

	// create metrics session
	sessionMetrics, err := app.CreateSession(config, metricsChannel)
	if err != nil {
		logger.Fatal("Failed to create metrics session", zap.Error(err))
	}
	err = sessionMetrics.Invite(exporterName)
	if err != nil {
		logger.Fatal("Failed to invite exporter to metrics session", zap.Error(err))
	}
	time.Sleep(500 * time.Millisecond)
	wg.Add(1)
	go handleSession(ctx, wg, logger, app, sessionMetrics, common.SignalMetrics)
	// create logs session
	sessionLogs, err := app.CreateSession(config, logsChannel)
	if err != nil {
		logger.Fatal("Failed to create logs session", zap.Error(err))
	}
	err = sessionLogs.Invite(exporterName)
	if err != nil {
		logger.Fatal("Failed to invite exporter to logs session", zap.Error(err))
	}
	time.Sleep(500 * time.Millisecond)
	wg.Add(1)
	go handleSession(ctx, wg, logger, app, sessionLogs, common.SignalLogs)

	logger.Info("Sessions created. Press Ctrl+C to stop")
}

// waitForSessionsAndMessages listens for incoming sessions and
// handles messages from each session concurrently
func waitForSessionsAndMessages(
	ctx context.Context,
	wg *sync.WaitGroup,
	logger *zap.Logger,
	app *slim.BindingsAdapter,
) {
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

			dstStr := dst.Components[0] + "/" + dst.Components[1] + "/" + dst.Components[2]

			logger.Info("New session established", zap.String("telemetryType", string(signalType)), zap.String("channelName", dstStr))
			// Handle the session in a goroutine
			wg.Add(1)
			go handleSession(ctx, wg, logger, app, session, signalType)
		}
	}
}

// handleSession processes messages from a single session
func handleSession(
	ctx context.Context,
	wg *sync.WaitGroup,
	logger *zap.Logger,
	app *slim.BindingsAdapter,
	session *slim.BindingsSessionContext,
	signalType common.SignalType,
) {
	defer wg.Done()

	sessionNum, err := session.SessionId()
	if err != nil {
		logger.Error("error getting session ID", zap.Error(err))
		return
	}

	defer func() {

		if err := app.DeleteSession(session); err != nil {
			logger.Warn("failed to delete session",
				zap.Uint32("sessionId", sessionNum),
				zap.String("signalType", string(signalType)),
				zap.Error(err))
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
			timeout := uint32(1000) // 1 sec
			msg, err := session.GetMessage(&timeout)
			if err != nil {
				errMsg := err.Error()
				switch {
				case strings.Contains(errMsg, "session closed"):
					logger.Info("Session closed by peer", zap.Uint32("sessionId", sessionNum), zap.Error(err))
					return
				case strings.Contains(errMsg, "receive timeout waiting for message"):
					// Normal timeout, continue
					continue
				default:
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
