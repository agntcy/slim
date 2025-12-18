package slimexporter

import (
	"context"
	"fmt"
	"strings"
	"sync"
	"time"

	slim "github.com/agntcy/slim/bindings/generated/slim_bindings"
	common "github.com/agntcy/slim/otel"

	"go.opentelemetry.io/collector/component"
	"go.opentelemetry.io/collector/pdata/plog"
	"go.opentelemetry.io/collector/pdata/pmetric"
	"go.opentelemetry.io/collector/pdata/ptrace"
	"go.uber.org/zap"
)

type SignalType string

const (
	SignalTraces  SignalType = "traces"
	SignalMetrics SignalType = "metrics"
	SignalLogs    SignalType = "logs"
)

const (
	inviteDelayMs     = 1000
	sessionTimeoutMs  = 1000
	defaultMaxRetries = 10
	defaultIntervalMs = 1000
)

// SignalSessions holds sessions related to a specific signal type
type SignalSessions struct {
	mutex    sync.RWMutex
	sessions map[uint32]*slim.BindingsSessionContext
}

func (s *SignalSessions) AddSession(session *slim.BindingsSessionContext) error {
	s.mutex.Lock()
	defer s.mutex.Unlock()
	if s.sessions == nil {
		s.sessions = make(map[uint32]*slim.BindingsSessionContext)
	}
	id, err := session.SessionId()
	if err != nil {
		return fmt.Errorf("session id is not set")
	}
	s.sessions[id] = session
	return nil
}

func (s *SignalSessions) RemoveSession(id uint32) error {
	s.mutex.Lock()
	defer s.mutex.Unlock()
	if s.sessions == nil {
		return fmt.Errorf("sessions map is nil")
	}
	if _, exists := s.sessions[id]; !exists {
		return fmt.Errorf("session with id %d not found", id)
	}
	delete(s.sessions, id)
	return nil
}

// PublishToAll publishes data to all sessions and returns a list of closed session IDs
func (s *SignalSessions) PublishToAll(data []byte, logger *zap.Logger, signalName string) ([]uint32, error) {
	s.mutex.RLock()
	defer s.mutex.RUnlock()

	var closedSessions []uint32
	for id, session := range s.sessions {
		if err := session.Publish(data, nil, nil); err != nil {
			if strings.Contains(err.Error(), "session closed") {
				logger.Info("Session closed, marking for removal", zap.Uint32("session_id", id))
				closedSessions = append(closedSessions, id)
				continue
			}
			logger.Error("Error sending "+signalName+" message", zap.Error(err))
			return closedSessions, err
		}
		logger.Debug("Published "+signalName+" to session", zap.Uint32("session_id", id))
	}

	return closedSessions, nil
}

// ExporterSessions holds session available in the
// exporter for each signal type
type ExporterSessions struct {
	app             *slim.BindingsAdapter
	connID          uint64
	metricsSessions *SignalSessions
	tracesSessions  *SignalSessions
	logsSessions    *SignalSessions
}

// AddSessionForSignal adds a session to the appropriate signal type's session list
func (e *ExporterSessions) AddSessionForSignal(signalType SignalType, session *slim.BindingsSessionContext) error {
	switch signalType {
	case SignalMetrics:
		return e.metricsSessions.AddSession(session)
	case SignalTraces:
		return e.tracesSessions.AddSession(session)
	case SignalLogs:
		return e.logsSessions.AddSession(session)
	default:
		return fmt.Errorf("unknown signal type: %s", signalType)
	}
}

// RemoveSessionForSignal removes a session from the appropriate signal type's session list
func (e *ExporterSessions) RemoveSessionForSignal(signalType SignalType, id uint32) error {
	switch signalType {
	case SignalMetrics:
		return e.metricsSessions.RemoveSession(id)
	case SignalTraces:
		return e.tracesSessions.RemoveSession(id)
	case SignalLogs:
		return e.logsSessions.RemoveSession(id)
	default:
		return fmt.Errorf("unknown signal type: %s", signalType)
	}
}

// RemoveAllSessionsForSignal removes all sessions for a specific signal type
func (e *ExporterSessions) RemoveAllSessionsForSignal(signalType SignalType) {
	var sessions *SignalSessions
	switch signalType {
	case SignalMetrics:
		sessions = e.metricsSessions
	case SignalTraces:
		sessions = e.tracesSessions
	case SignalLogs:
		sessions = e.logsSessions
	default:
		return
	}

	sessions.mutex.Lock()
	defer sessions.mutex.Unlock()
	for id, session := range sessions.sessions {
		e.app.DeleteSession(session)
		delete(sessions.sessions, id)
	}
}

var (
	// used to settup app and connID only once
	mutex               sync.Mutex
	state               *ExporterSessions
	listenerStartedOnce sync.Once
)

// slimExporter implements the exporter for traces, metrics, and logs
type slimExporter struct {
	config     *Config
	logger     *zap.Logger
	signalType SignalType
	sessions   *ExporterSessions
}

// getOrCreateApp creates or retrieves a shared slim application and connection ID
func getOrCreateApp(localID string, serverAddr string, secret string) (*slim.BindingsAdapter, uint64, error) {
	mutex.Lock()
	defer mutex.Unlock()

	// If connection exists and matches config, reuse it
	if state != nil {
		return state.app, state.connID, nil
	}

	app, connID, err := common.CreateAndConnectApp(localID, serverAddr, secret)
	if err != nil {
		return nil, 0, fmt.Errorf("failed to create/connect app:: %w", err)
	}

	// Store shared connection
	state = &ExporterSessions{
		app:             app,
		connID:          connID,
		metricsSessions: &SignalSessions{},
		tracesSessions:  &SignalSessions{},
		logsSessions:    &SignalSessions{},
	}

	return app, connID, nil
}

// createSessionAndInvite creates a session for the given channel and signal,
// and invites the participants specified in the config
func createSessionAndInvite(app *slim.BindingsAdapter, connID uint64, config SessionConfig, channel string, signal string) (*slim.BindingsSessionContext, error) {
	channel = fmt.Sprintf("%s-%s", channel, signal)
	name, err := common.SplitID(channel)
	if err != nil {
		return nil, fmt.Errorf("failed to parse channel name: %w", err)
	}

	maxRetries := uint32(defaultMaxRetries)
	intervalMs := uint64(defaultIntervalMs)
	sessionConfig := slim.SessionConfig{
		SessionType: slim.SessionTypeGroup,
		EnableMls:   config.MlsEnabled,
		MaxRetries:  &maxRetries,
		IntervalMs:  &intervalMs,
		Initiator:   true,
	}

	session, err := app.CreateSession(sessionConfig, name)
	if err != nil {
		return nil, fmt.Errorf("failed to create the session: %w", err)
	}

	for _, participant := range config.Participants {
		participantName, err := common.SplitID(participant)
		if err != nil {
			return nil, fmt.Errorf("failed to parse participant name %s: %w", participant, err)
		}
		app.SetRoute(participantName, connID)
		if err := session.Invite(participantName); err != nil {
			return nil, fmt.Errorf("failed to invite participant %s: %w", participant, err)
		}
		time.Sleep(inviteDelayMs * time.Millisecond)
	}

	return session, nil
}

// listenForAllSessions is a shared function that listens for all incoming sessions
// and distributes them to the appropriate exporters based on the session name suffix
func listenForAllSessions(ctx context.Context, app *slim.BindingsAdapter, logger *zap.Logger) {
	logger.Info("Listener started, waiting for incoming sessions...")

	for {
		select {
		case <-ctx.Done():
			logger.Info("Shutting down listener...")
			return

		default:
			timeout := uint32(sessionTimeoutMs)
			session, err := app.ListenForSession(&timeout)
			if err != nil {
				logger.Debug("Timeout waiting for session, retrying...")
				continue
			}

			logger.Info("New session established!")

			// Parse the session name to determine which channel it belongs to
			sessionName, err := session.Destination()
			if err != nil {
				logger.Error("Failed to get session destination", zap.Error(err))
				continue
			}

			// Check the third component (index 2) of the session name
			if len(sessionName.Components) < 3 {
				logger.Error("Received session with invalid name structure", zap.Int("components_count", len(sessionName.Components)))
				continue
			}

			channelComponent := sessionName.Components[2]

			// Determine signal type from suffix and assign to appropriate exporter
			var assignedSignal SignalType
			if strings.HasSuffix(channelComponent, string(SignalMetrics)) {
				assignedSignal = SignalMetrics
			} else if strings.HasSuffix(channelComponent, string(SignalTraces)) {
				assignedSignal = SignalTraces
			} else if strings.HasSuffix(channelComponent, string(SignalLogs)) {
				assignedSignal = SignalLogs
			} else {
				logger.Error("Received session with unrecognized suffix", zap.String("session_name", channelComponent))
				continue
			}

			logger.Info("New session received for signal",
				zap.String("signal", string(assignedSignal)),
				zap.String("session_name", channelComponent))

			// Store session in exporter sessions
			err = state.AddSessionForSignal(assignedSignal, session)
			if err != nil {
				logger.Error("Failed to add session", zap.String("signal", string(assignedSignal)), zap.Error(err))
				continue
			}
		}
	}
}

// newSlimExporter creates a new instance of the slim exporter
func newSlimExporter(cfg *Config, logger *zap.Logger, signalType SignalType) (*slimExporter, error) {

	app, connID, err := getOrCreateApp(cfg.LocalName, cfg.SlimEndpoint, cfg.SharedSecret)
	if err != nil {
		return nil, fmt.Errorf("failed to create/connect app: %w", err)
	}

	for _, sessionCfg := range cfg.Sessions {
		for _, singal := range sessionCfg.Signals {
			if singal == string(signalType) {
				session, err := createSessionAndInvite(app, connID, sessionCfg, sessionCfg.ChannelName, singal)
				if err != nil {
					return nil, fmt.Errorf("failed to create and invite session for channel %s and signal %s: %w", sessionCfg.ChannelName, singal, err)
				}
				logger.Info("Session created and participants invited",
					zap.String("channel", sessionCfg.ChannelName),
					zap.String("signal", singal))

				// Store session in exporter sessions
				err = state.AddSessionForSignal(signalType, session)
				if err != nil {
					return nil, fmt.Errorf("failed to add %s session: %w", signalType, err)
				}
			}
		}
	}

	slim := &slimExporter{
		config:     cfg,
		logger:     logger,
		signalType: signalType,
		sessions:   state,
	}

	return slim, nil
}

// start is invoked during service startup
func (e *slimExporter) start(ctx context.Context, host component.Host) error {
	e.logger.Info("Starting Slim exporter",
		zap.String("endpoint", e.config.SlimEndpoint),
		zap.String("local-name", e.config.LocalName),
		zap.String("signal", string(e.signalType)))

	// if the session reception loop is not started yet, start it
	listenerStartedOnce.Do(func() {
		e.logger.Info("Starting shared session listener")
		go listenForAllSessions(ctx, e.sessions.app, e.logger)
	})

	return nil
}

// shutdown is invoked during service shutdown
func (e *slimExporter) shutdown(ctx context.Context) error {
	e.logger.Info("Shutting down Slim exporter", zap.String("signal", string(e.signalType)))

	// Update shared data and close the app if all sessions are nil
	if state != nil {
		state.RemoveAllSessionsForSignal(e.signalType)

		// if all sessions are closed, destroy the app
		allMetricsClosed := len(state.metricsSessions.sessions) == 0
		allTracesClosed := len(state.tracesSessions.sessions) == 0
		allLogsClosed := len(state.logsSessions.sessions) == 0

		if allMetricsClosed && allTracesClosed && allLogsClosed {
			e.logger.Info("All sessions closed, destroying application")
			// this is safe as only one exporter can reach this point
			e.sessions.app.Destroy()
			state = nil
		}
	}

	return nil
}

// pushTraces exports trace data
func (e *slimExporter) pushTraces(ctx context.Context, td ptrace.Traces) error {
	marshaler := ptrace.ProtoMarshaler{}
	message, err := marshaler.MarshalTraces(td)
	if err != nil {
		e.logger.Error("Failed to marshal traces to OTLP format", zap.Error(err))
		return err
	}

	return e.publishData(message, "traces", td.SpanCount())
}

// pushMetrics exports metrics data
func (e *slimExporter) pushMetrics(ctx context.Context, md pmetric.Metrics) error {
	marshaler := pmetric.ProtoMarshaler{}
	message, err := marshaler.MarshalMetrics(md)
	if err != nil {
		e.logger.Error("Failed to marshal metrics to OTLP format", zap.Error(err))
		return err
	}

	return e.publishData(message, "metrics", md.DataPointCount())
}

// pushLogs exports logs data
func (e *slimExporter) pushLogs(ctx context.Context, ld plog.Logs) error {
	marshaler := plog.ProtoMarshaler{}
	message, err := marshaler.MarshalLogs(ld)
	if err != nil {
		e.logger.Error("Failed to marshal logs to OTLP format", zap.Error(err))
		return err
	}

	return e.publishData(message, "logs", ld.LogRecordCount())
}

// send data to all sessions related to the signal type
func (e *slimExporter) publishData(data []byte, signalName string, count int) error {
	e.logger.Info("Exporting "+signalName,
		zap.Int("count", count),
		zap.String("endpoint", e.config.SlimEndpoint))

	var closedSessions []uint32
	var err error
	var signalType SignalType

	switch signalName {
	case "traces":
		closedSessions, err = state.tracesSessions.PublishToAll(data, e.logger, signalName)
		signalType = SignalTraces
	case "metrics":
		closedSessions, err = state.metricsSessions.PublishToAll(data, e.logger, signalName)
		signalType = SignalMetrics
	case "logs":
		closedSessions, err = state.logsSessions.PublishToAll(data, e.logger, signalName)
		signalType = SignalLogs
	default:
		e.logger.Error("Unknown signal type: " + signalName)
		return fmt.Errorf("unknown signal type: %s", signalName)
	}

	if err != nil {
		return err
	}

	// Remove closed sessions after iteration
	for _, id := range closedSessions {
		state.RemoveSessionForSignal(signalType, id)
	}

	return nil
}
