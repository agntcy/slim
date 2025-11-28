package slimexporter

import (
	"context"
	"fmt"
	"strings"
	"sync"
	"time"

	slim "github.com/agntcy/slim/bindings/generated/slim_service"
	"github.com/agntcy/slim/bindings/go/examples/common"

	"go.opentelemetry.io/collector/component"
	"go.opentelemetry.io/collector/consumer"
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
	publishFanout     = 256
	inviteDelayMs     = 500
	sessionTimeoutMs  = 30000
	defaultMaxRetries = 10
	defaultIntervalMs = 1000
)

// sharedStructure holds the info that are init
// once and shared on the different slimExporter
// one for each metric type
type sharedStructure struct {
	app            *slim.BindingsAdapter
	channelName    *slim.Name
	connID         uint64
	metricsChannel *slim.FfiSessionContext
	tracesChannel  *slim.FfiSessionContext
	logsChannel    *slim.FfiSessionContext
}

func (s *sharedStructure) getChannel(signalType SignalType) *slim.FfiSessionContext {
	switch signalType {
	case SignalMetrics:
		return s.metricsChannel
	case SignalTraces:
		return s.tracesChannel
	case SignalLogs:
		return s.logsChannel
	default:
		return nil
	}
}

func (s *sharedStructure) setChannel(signalType SignalType, channel *slim.FfiSessionContext) {
	switch signalType {
	case SignalMetrics:
		s.metricsChannel = channel
	case SignalTraces:
		s.tracesChannel = channel
	case SignalLogs:
		s.logsChannel = channel
	}
}

func (s *sharedStructure) allChannelsNil() bool {
	return s.metricsChannel == nil && s.tracesChannel == nil && s.logsChannel == nil
}

func (s *sharedStructure) allChannelsSet() bool {
	return s.metricsChannel != nil && s.tracesChannel != nil && s.logsChannel != nil
}

var (
	sharedData          *sharedStructure
	connMu              sync.Mutex
	listenerStartedOnce sync.Once
)

// slimExporter implements the exporter for traces, metrics, and logs
type slimExporter struct {
	config      *Config
	logger      *zap.Logger
	signalType  SignalType
	app         *slim.BindingsAdapter
	channelName *slim.Name

	connId  uint64
	channel *slim.FfiSessionContext
}

func getOrCreateSharedData(localID, serverAddr, secret, channelName string) (*slim.BindingsAdapter, *slim.Name, uint64, error) {
	connMu.Lock()
	defer connMu.Unlock()

	// If connection exists and matches config, reuse it
	if sharedData != nil {
		return sharedData.app, sharedData.channelName, sharedData.connID, nil
	}

	app, connID, err := common.CreateAndConnectApp(localID, serverAddr, secret)
	if err != nil {
		return nil, nil, 0, fmt.Errorf("failed to create/connect app:: %w", err)
	}

	// Parse channel name
	chanName, err := common.SplitID(channelName)
	if err != nil {
		app.Destroy()
		return nil, nil, 0, fmt.Errorf("invalid channel name: %w", err)
	}

	// Store shared connection
	sharedData = &sharedStructure{
		app:            app,
		channelName:    &chanName,
		connID:         connID,
		metricsChannel: nil,
		tracesChannel:  nil,
		logsChannel:    nil,
	}

	return app, &chanName, connID, nil
}

func createSession(e *slimExporter, config slim.SessionConfig, channel string) (*slim.FfiSessionContext, error) {
	name, err := common.SplitID(channel)
	if err != nil {
		return nil, fmt.Errorf("failed to parse channel name: %w", err)
	}
	session, err := e.app.CreateSession(config, name)
	if err != nil {
		return nil, fmt.Errorf("failed to create channel: %w", err)
	}

	return session, nil
}

// listenForAllSessions is a shared function that listens for all incoming sessions
// and distributes them to the appropriate exporters based on the session name suffix
func listenForAllSessions(app *slim.BindingsAdapter, logger *zap.Logger) {
	logger.Info("Shared session listener started, waiting for incoming sessions...")

	for {
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
			logger.Warn("Received session with invalid name structure", zap.Int("components_count", len(sessionName.Components)))
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
			logger.Warn("Received session with unrecognized suffix", zap.String("session_name", channelComponent))
			continue
		}

		connMu.Lock()
		done := false
		if sharedData != nil {
			sharedData.setChannel(assignedSignal, session)
			done = sharedData.allChannelsSet()
		}
		connMu.Unlock()

		if done {
			logger.Info("All registered exporters have been assigned sessions, stopping listener")
			break
		}
	}
}

// newSlimExporter creates a new instance of the slim exporter
func newSlimExporter(cfg *Config, logger *zap.Logger, signalType SignalType) (*slimExporter, error) {
	app, channelName, connId, err := getOrCreateSharedData(
		cfg.LocalName,
		cfg.SlimEndpoint,
		cfg.SharedSecret,
		cfg.ChannelName)

	if err != nil {
		return nil, fmt.Errorf("failed to create/connect app: %w", err)
	}

	slim := &slimExporter{
		config:      cfg,
		logger:      logger,
		signalType:  signalType,
		app:         app,
		channelName: channelName,
		connId:      connId,
		channel:     nil,
	}

	return slim, nil
}

// start is invoked during service startup
func (e *slimExporter) start(ctx context.Context, host component.Host) error {
	e.logger.Info("Starting Slim exporter",
		zap.String("endpoint", e.config.SlimEndpoint),
		zap.String("local-name", e.config.LocalName),
		zap.String("channel-name", e.config.ChannelName),
		zap.String("signal", string(e.signalType)))

	// If no participants to invite, wait for incoming session invitations
	if len(e.config.ParticipantsList) == 0 {
		e.logger.Info("No participants to invite, waiting for incoming sessions...",
			zap.String("signal", string(e.signalType)))

		listenerStartedOnce.Do(func() {
			e.logger.Info("Starting shared session listener")
			go listenForAllSessions(e.app, e.logger)
		})

		return nil
	}

	toInvite, err := e.parseParticipants()
	if err != nil {
		return fmt.Errorf("failed to parse participants: %w", err)
	}

	return e.createAndInviteSession(toInvite)
}

func (e *slimExporter) parseParticipants() ([]slim.Name, error) {
	toInvite := make([]slim.Name, 0, len(e.config.ParticipantsList))
	for _, p := range e.config.ParticipantsList {
		name, err := common.SplitID(p)
		if err != nil {
			return nil, fmt.Errorf("failed to parse remote ID %s: %w", p, err)
		}
		toInvite = append(toInvite, name)
		e.app.SetRoute(name, e.connId)
	}
	return toInvite, nil
}

func (e *slimExporter) createAndInviteSession(toInvite []slim.Name) error {
	fullName := fmt.Sprintf("%s-%s", e.config.ChannelName, e.signalType)
	e.logger.Info("Creating channel",
		zap.String("signal", string(e.signalType)),
		zap.String("channel-name", fullName))

	// Invite all the participants to the session related to this signal type
	maxRetries := uint32(defaultMaxRetries)
	intervalMs := uint64(defaultIntervalMs)
	config := slim.SessionConfig{
		SessionType: slim.SessionTypeMulticast,
		EnableMls:   e.config.MlsEnabled,
		MaxRetries:  &maxRetries,
		IntervalMs:  &intervalMs,
		Initiator:   true,
	}

	session, err := createSession(e, config, fullName)
	if err != nil {
		return fmt.Errorf("failed to create session: %w", err)
	}
	e.channel = session

	// Invite all participants
	for _, p := range toInvite {
		if err := e.channel.Invite(p); err != nil {
			return fmt.Errorf("failed to invite participant: %w", err)
		}
		time.Sleep(inviteDelayMs * time.Millisecond)
	}

	// Add the session to the shared data
	connMu.Lock()
	if sharedData != nil {
		sharedData.setChannel(e.signalType, session)
	}
	connMu.Unlock()

	return nil
}

// shutdown is invoked during service shutdown
func (e *slimExporter) shutdown(ctx context.Context) error {
	e.logger.Info("Shutting down Slim exporter", zap.String("signal", string(e.signalType)))

	connMu.Lock()
	defer connMu.Unlock()

	// Update shared data and close the app if all sessions are nil
	if sharedData != nil {
		session := sharedData.getChannel(e.signalType)

		if session != nil {
			e.app.DeleteSession(session)
			sharedData.setChannel(e.signalType, nil)
		}

		if sharedData.allChannelsNil() {
			e.logger.Info("All channels closed, destroying application")
			sharedData.app.Destroy()
			sharedData = nil
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

// publishData is a generic method to publish telemetry data
func (e *slimExporter) publishData(data []byte, signalName string, count int) error {
	e.logger.Info("Exporting "+signalName,
		zap.Int("count", count),
		zap.String("endpoint", e.config.SlimEndpoint))

	channel := e.getOrFetchChannel()
	if channel == nil {
		e.logger.Info(signalName + " channel not set yet, dropping message")
		return nil
	}

	if err := channel.Publish(data, nil, nil); err != nil {
		e.logger.Error("Error sending "+signalName+" message", zap.Error(err))
		return err
	}

	return nil
}

// getOrFetchChannel retrieves the channel, fetching from shared data if necessary
func (e *slimExporter) getOrFetchChannel() *slim.FfiSessionContext {
	if e.channel != nil {
		return e.channel
	}

	connMu.Lock()
	if sharedData != nil {
		e.channel = sharedData.getChannel(e.signalType)
	}
	connMu.Unlock()

	return e.channel
}

// Capabilities returns the consumer capabilities of the exporter
func (e *slimExporter) Capabilities() consumer.Capabilities {
	return consumer.Capabilities{MutatesData: false}
}
