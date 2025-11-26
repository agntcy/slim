package slimexporter

import (
	"context"
	"fmt"
	"log"
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

// sharedConnection holds the singleton SLIM connection
type sharedConnection struct {
	mu          sync.Mutex
	app         *slim.BindingsAdapter
	channelName *slim.Name
	connID      uint64
	config      connectionConfig
	refCount    int
}

type connectionConfig struct {
	localID     string
	serverAddr  string
	secret      string
	channelName string
}

var (
	sharedConn *sharedConnection
	connMu     sync.Mutex
)

// slimExporter implements the exporter for traces, metrics, and logs
type slimExporter struct {
	config         *Config
	logger         *zap.Logger
	signalType     SignalType
	app            *slim.BindingsAdapter
	channelName    *slim.Name
	connId         uint64
	metricsChannel *slim.FfiSessionContext
	tracesChannel  *slim.FfiSessionContext
	logsChannel    *slim.FfiSessionContext
}

func getOrCreateSharedConnection(localID, serverAddr, secret, channelName string) (*slim.BindingsAdapter, *slim.Name, uint64, error) {
	connMu.Lock()
	defer connMu.Unlock()

	// If connection exists and matches config, reuse it
	if sharedConn != nil {
		if sharedConn.config.localID == localID &&
			sharedConn.config.serverAddr == serverAddr &&
			sharedConn.config.secret == secret &&
			sharedConn.config.channelName == channelName {
			sharedConn.refCount++
			return sharedConn.app, sharedConn.channelName, sharedConn.connID, nil
		}
		// Config mismatch - shouldn't happen in normal operation
		return nil, nil, 0, fmt.Errorf("connection config mismatch")
	}

	// Create new connection
	slim.InitializeCrypto()

	appName, err := common.SplitID(localID)
	if err != nil {
		return nil, nil, 0, fmt.Errorf("invalid local ID: %w", err)
	}

	app, err := slim.CreateAppWithSecret(appName, secret)
	if err != nil {
		return nil, nil, 0, fmt.Errorf("create app failed: %w", err)
	}

	// Connect to SLIM server (returns connection ID)
	config := slim.ClientConfig{
		Endpoint: serverAddr,
		Tls:      slim.TlsConfig{Insecure: true},
	}
	connID, err := app.Connect(config)
	if err != nil {
		app.Destroy()
		return nil, nil, 0, fmt.Errorf("connect failed: %w", err)
	}

	// Parse channel name
	chanName, err := common.SplitID(channelName)
	if err != nil {
		app.Destroy()
		return nil, nil, 0, fmt.Errorf("invalid channel name: %w", err)
	}

	// Store shared connection
	sharedConn = &sharedConnection{
		app:         app,
		channelName: &chanName,
		connID:      connID,
		config: connectionConfig{
			localID:     localID,
			serverAddr:  serverAddr,
			secret:      secret,
			channelName: channelName,
		},
		refCount: 1,
	}

	return app, &chanName, connID, nil
}

func releaseSharedConnection() {
	connMu.Lock()
	defer connMu.Unlock()

	if sharedConn == nil {
		return
	}

	sharedConn.refCount--
	if sharedConn.refCount <= 0 {
		// Clean up connection
		if sharedConn.app != nil {
			sharedConn.app.Destroy()
		}
		sharedConn = nil
	}
}

func createSession(e *slimExporter, config slim.SessionConfig, channel string) (*slim.FfiSessionContext, error) {
	name, err := common.SplitID(channel)
	if err != nil {
		log.Fatalf("Failed to parse channel name: %v", err)
		return nil, err
	}
	session, err := e.app.CreateSession(config, name)
	if err != nil {
		log.Fatalf("Failed to create channel: %v", err)
		return nil, err
	}

	return session, nil
}

// newSlimExporter creates a new instance of the slim exporter
func newSlimExporter(cfg *Config, logger *zap.Logger, signalType SignalType) (*slimExporter, error) {
	app, channelName, connId, err := getOrCreateSharedConnection(
		cfg.LocalName,
		cfg.SlimEndpoint,
		cfg.SharedSecret,
		cfg.ChannelName)

	if err != nil {
		log.Fatalf("Failed to create/connect app: %v", err)
		return nil, err
	}

	slim := &slimExporter{
		config:         cfg,
		logger:         logger,
		signalType:     signalType,
		app:            app,
		channelName:    channelName,
		connId:         connId,
		metricsChannel: nil,
		tracesChannel:  nil,
		logsChannel:    nil,
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

	config := slim.SessionConfig{
		SessionType: slim.SessionTypeMulticast,
		EnableMls:   false, // need to be set in the config
	}

	toInvite := []slim.Name{}
	for _, p := range e.config.ParticipantsList {
		name, err := common.SplitID(p)
		if err != nil {
			log.Fatalf("Failed to parse remote ID: %v", err)
			return err
		}
		toInvite = append(toInvite, name)
		e.app.SetRoute(name, e.connId)
	}

	// If no participants to invite, wait for incoming session invitations
	if len(toInvite) == 0 {
		e.logger.Info("No participants to invite, waiting for incoming sessions...",
			zap.String("signal", string(e.signalType)))
		go e.listenForSessions()
		return nil
	}

	// create the session according to the signal type
	switch e.signalType {
	case SignalMetrics:
		fullName := e.config.ChannelName + "-metrics"
		e.logger.Info("Create channel for metrics", zap.String("channel-name", fullName))
		session, err := createSession(e, config, fullName)
		if err != nil {
			log.Fatalf("Failed to create channel: %v", err)
			return err
		}
		e.metricsChannel = session
		for _, p := range toInvite {
			e.metricsChannel.Invite(p)
			time.Sleep(500 * time.Millisecond)
		}
	case SignalTraces:
		fullName := e.config.ChannelName + "-traces"
		e.logger.Info("Create channel for traces", zap.String("channel-name", fullName))
		session, err := createSession(e, config, fullName)
		if err != nil {
			log.Fatalf("Failed to create channel: %v", err)
			return err
		}
		e.tracesChannel = session
		for _, p := range toInvite {
			e.tracesChannel.Invite(p)
			time.Sleep(500 * time.Millisecond)
		}
	case SignalLogs:
		fullName := e.config.ChannelName + "-logs"
		e.logger.Info("Create channel for logs", zap.String("channel-name", fullName))
		session, err := createSession(e, config, fullName)
		if err != nil {
			log.Fatalf("Failed to create channel: %v", err)
			return err
		}
		e.logsChannel = session
		for _, p := range toInvite {
			e.logsChannel.Invite(p)
			time.Sleep(500 * time.Millisecond)
		}
	}

	return nil
}

// listenForSessions waits for incoming session invitations and assigns them to the appropriate channel
func (e *slimExporter) listenForSessions() {
	e.logger.Info("Waiting for incoming sessions...", zap.String("signal", string(e.signalType)))

	/*for {
		timeout := uint32(30000)                         // 30 seconds
		session, err := e.app.ListenForSession(&timeout) // needs to be optional
		if err != nil {
			e.logger.Info("Timeout waiting for session, retrying...", zap.String("signal", string(e.signalType)))
			continue
		}

		e.logger.Info("New session established!", zap.String("signal", string(e.signalType)))

		// Parse the session name to determine which channel it belongs to
		sessionName := session.GetName()
		sessionNameStr := fmt.Sprintf("%s.%s", sessionName.Namespace, sessionName.Name)

		// Assign the session to the appropriate channel based on the signal type suffix
		if len(sessionNameStr) >= 8 && sessionNameStr[len(sessionNameStr)-8:] == "-metrics" {
			e.metricsChannel = session
			e.logger.Info("Assigned incoming session to metrics channel", zap.String("session_name", sessionNameStr))
		} else if len(sessionNameStr) >= 7 && sessionNameStr[len(sessionNameStr)-7:] == "-traces" {
			e.tracesChannel = session
			e.logger.Info("Assigned incoming session to traces channel", zap.String("session_name", sessionNameStr))
		} else if len(sessionNameStr) >= 5 && sessionNameStr[len(sessionNameStr)-5:] == "-logs" {
			e.logsChannel = session
			e.logger.Info("Assigned incoming session to logs channel", zap.String("session_name", sessionNameStr))
		} else {
			e.logger.Warn("Received session with unrecognized name format", zap.String("session_name", sessionNameStr))
		}

		// After assigning the session for this signal type, we can break
		// since each exporter instance handles only one signal type
		break
	}*/
}

// shutdown is invoked during service shutdown
func (e *slimExporter) shutdown(ctx context.Context) error {
	e.logger.Info("Shutting down Slim exporter", zap.String("signal", string(e.signalType)))

	// close related channel if set
	switch e.signalType {
	case SignalMetrics:
		if e.metricsChannel != nil {
			e.logger.Info("Delete channel for metrics")
			e.app.DeleteSession(e.metricsChannel)
		}
	case SignalTraces:
		if e.tracesChannel != nil {
			e.logger.Info("Delete channel for traces")
			e.app.DeleteSession(e.tracesChannel)
		}
	case SignalLogs:
		if e.logsChannel != nil {
			e.logger.Info("Delete channel for logs")
			e.app.DeleteSession(e.logsChannel)
		}
	}

	// Release the shared connection (will only destroy when refCount reaches 0)
	releaseSharedConnection()

	return nil
}

// pushTraces exports trace data
func (e *slimExporter) pushTraces(ctx context.Context, td ptrace.Traces) error {
	// Implement your trace export logic here
	spanCount := td.SpanCount()
	e.logger.Info("Exporting traces",
		zap.Int("span_count", spanCount),
		zap.String("endpoint", e.config.SlimEndpoint))

	// Serialize traces to OTLP protobuf format
	marshaler := ptrace.ProtoMarshaler{}
	message, err := marshaler.MarshalTraces(td)
	if err != nil {
		e.logger.Error("Failed to marshal traces to OTLP format", zap.Error(err))
		return err
	}

	if err := e.tracesChannel.Publish(*e.channelName, 256, message, nil, nil, nil); err != nil {
		e.logger.Error("Error sending trace message", zap.Error(err))
		return err
	}

	return nil
}

// pushMetrics exports metrics data
func (e *slimExporter) pushMetrics(ctx context.Context, md pmetric.Metrics) error {
	dataPointCount := md.DataPointCount()
	e.logger.Info("Exporting metrics",
		zap.Int("data_point_count", dataPointCount),
		zap.String("endpoint", e.config.SlimEndpoint))

	// Serialize metrics to OTLP protobuf format
	marshaler := pmetric.ProtoMarshaler{}
	message, err := marshaler.MarshalMetrics(md)
	if err != nil {
		e.logger.Error("Failed to marshal metrics to OTLP format", zap.Error(err))
		return err
	}

	if err := e.metricsChannel.Publish(*e.channelName, 256, message, nil, nil, nil); err != nil {
		e.logger.Error("Error sending metrics message", zap.Error(err))
		return err
	}

	return nil
}

// pushLogs exports logs data
func (e *slimExporter) pushLogs(ctx context.Context, ld plog.Logs) error {
	logRecordCount := ld.LogRecordCount()
	e.logger.Info("Exporting logs",
		zap.Int("log_record_count", logRecordCount),
		zap.String("endpoint", e.config.SlimEndpoint))

	// Serialize logs to OTLP protobuf format
	marshaler := plog.ProtoMarshaler{}
	message, err := marshaler.MarshalLogs(ld)
	if err != nil {
		e.logger.Error("Failed to marshal logs to OTLP format", zap.Error(err))
		return err
	}

	if err := e.logsChannel.Publish(*e.channelName, 256, message, nil, nil, nil); err != nil {
		e.logger.Error("Error sending logs message", zap.Error(err))
		return err
	}

	return nil
}

// Capabilities returns the consumer capabilities of the exporter
func (e *slimExporter) Capabilities() consumer.Capabilities {
	return consumer.Capabilities{MutatesData: false}
}
