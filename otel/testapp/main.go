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
	"runtime"
	"syscall"
	"time"

	slim "github.com/agntcy/slim/bindings/generated/slim_bindings"
	common "github.com/agntcy/slim/otel"
	"go.opentelemetry.io/collector/pdata/plog"
	"go.opentelemetry.io/collector/pdata/pmetric"
	"go.opentelemetry.io/collector/pdata/ptrace"
)

func main() {
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

	if *exporterNameStr == "" {
		waitForSessionsAndMessages(app, ctx)
	} else {
		initiateSessions(ctx, app, exporterNameStr, *mlsEnabled, connId)
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

	fmt.Printf("Create session and invite %s\n", exporterNameStr)

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
	go handleSession(ctx, app, sessionTraces, common.SignalTraces)

	// create metrics session
	sessionMetrics, err := app.CreateSession(config, metricsChannel)
	if err != nil {
		log.Fatalf("Failed to create metrics session: %v", err)
	}
	sessionMetrics.Invite(exporterName)
	time.Sleep(500 * time.Millisecond)
	go handleSession(ctx, app, sessionMetrics, common.SignalMetrics)
	// create logs session
	sessionLogs, err := app.CreateSession(config, logsChannel)
	if err != nil {
		log.Fatalf("Failed to create logs session: %v", err)
	}
	sessionLogs.Invite(exporterName)
	time.Sleep(500 * time.Millisecond)
	go handleSession(ctx, app, sessionLogs, common.SignalLogs)

	fmt.Println("Sessions created. Press Ctrl+C to stop")

	// Wait for shutdown signal
	<-ctx.Done()
	fmt.Println("Shutting down...")
}

// waitForSessionsAndMessages listens for incoming sessions and handles messages from each session concurrently
func waitForSessionsAndMessages(app *slim.BindingsAdapter, ctx context.Context) {
	fmt.Println("Waiting for incoming sessions...")
	fmt.Println("Press Ctrl+C to stop")

	for {
		select {
		case <-ctx.Done():
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

			dst, err := session.Destination()
			if err != nil {
				log.Printf("error getting destination from new recevied sesison: %v", err)
				continue
			}

			if len(dst.Components) < 3 {
				log.Printf("session destination has insufficient components")
				continue
			}

			// Extract signal type from dst.Components[2] suffix
			telemetryType := dst.Components[2]
			signalType, err := common.ExtractSignalType(telemetryType)
			if err != nil {
				log.Printf("error extracting signal type: %v", err)
				continue
			}

			fmt.Printf("\nNew session established. Telemetry Type %s\n", signalType)
			// Handle the session in a goroutine
			go handleSession(ctx, app, session, signalType)
		}
	}
}

// handleSession processes messages from a single session
func handleSession(ctx context.Context, app *slim.BindingsAdapter, session *slim.BindingsSessionContext, signalType common.SignalType) {
	// Lock this goroutine to an OS thread to prevent FFI calls from blocking other goroutines
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()

	sessionNum, err := session.SessionId()
	if err != nil {
		log.Printf("error getting session ID: %v", err)
		return
	}

	defer func() {
		if err := app.DeleteSession(session); err != nil {
			log.Printf("[Session %d, %s] Warning: failed to delete session: %v", sessionNum, signalType, err)
		}
		fmt.Printf("[Session %d, %s] Session closed\n", sessionNum, signalType)
	}()

	messageCount := 0

	for {
		select {
		case <-ctx.Done():
			fmt.Printf("[Session %d, %s] Shutting down after %d messages\n", sessionNum, signalType, messageCount)
			return
		default:

			// Wait for message with timeout
			timeout := uint32(1000) // 1 sec
			msg, err := session.GetMessage(&timeout)
			if err != nil {
				// Timeout is normal, just continue
				continue
			}

			messageCount++
			fmt.Printf("[Session %d, %s] Received message %d (size: %d bytes)\n", sessionNum, signalType, messageCount, len(msg.Payload))

			// Parse and print OTLP data based on signal type
			switch signalType {
			case common.SignalTraces:
				unmarshaler := ptrace.ProtoUnmarshaler{}
				traces, err := unmarshaler.UnmarshalTraces(msg.Payload)
				if err != nil {
					log.Printf("[Session %d, traces] Error unmarshaling traces: %v", sessionNum, err)
					continue
				}
				fmt.Printf("[Session %d, traces] Message %d: received %d spans\n", sessionNum, messageCount, traces.SpanCount())

				// Print trace details
				for i := 0; i < traces.ResourceSpans().Len(); i++ {
					rs := traces.ResourceSpans().At(i)
					for j := 0; j < rs.ScopeSpans().Len(); j++ {
						ss := rs.ScopeSpans().At(j)
						for k := 0; k < ss.Spans().Len(); k++ {
							span := ss.Spans().At(k)
							fmt.Printf("  Span: %s (TraceID: %s, SpanID: %s)\n",
								span.Name(), span.TraceID(), span.SpanID())
						}
					}
				}

			case common.SignalMetrics:
				unmarshaler := pmetric.ProtoUnmarshaler{}
				metrics, err := unmarshaler.UnmarshalMetrics(msg.Payload)
				if err != nil {
					log.Printf("[Session %d, metrics] Error unmarshaling metrics: %v", sessionNum, err)
					continue
				}
				fmt.Printf("[Session %d, metrics] Message %d: received %d data points\n", sessionNum, messageCount, metrics.DataPointCount())

				// Print metric details
				for i := 0; i < metrics.ResourceMetrics().Len(); i++ {
					rm := metrics.ResourceMetrics().At(i)
					for j := 0; j < rm.ScopeMetrics().Len(); j++ {
						sm := rm.ScopeMetrics().At(j)
						for k := 0; k < sm.Metrics().Len(); k++ {
							metric := sm.Metrics().At(k)
							fmt.Printf("  Metric: %s (Type: %s)\n", metric.Name(), metric.Type())
						}
					}
				}

			case common.SignalLogs:
				unmarshaler := plog.ProtoUnmarshaler{}
				logs, err := unmarshaler.UnmarshalLogs(msg.Payload)
				if err != nil {
					log.Printf("[Session %d, logs] Error unmarshaling logs: %v", sessionNum, err)
					continue
				}
				fmt.Printf("[Session %d, logs] Message %d: received %d log records\n", sessionNum, messageCount, logs.LogRecordCount())

				// Print log details
				for i := 0; i < logs.ResourceLogs().Len(); i++ {
					rl := logs.ResourceLogs().At(i)
					for j := 0; j < rl.ScopeLogs().Len(); j++ {
						sl := rl.ScopeLogs().At(j)
						for k := 0; k < sl.LogRecords().Len(); k++ {
							log := sl.LogRecords().At(k)
							fmt.Printf("  Log: %s (Severity: %s)\n", log.Body().AsString(), log.SeverityText())
						}
					}
				}

			default:
				log.Printf("[Session %d] Unknown signal type: %s", sessionNum, signalType)
			}

			fmt.Println()
		}
	}
}
