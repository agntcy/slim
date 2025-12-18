package slimexporter

import (
	"context"
	"sync"
	"testing"

	slim "github.com/agntcy/slim/bindings/generated/slim_bindings"
	common "github.com/agntcy/slim/otel"

	"go.opentelemetry.io/collector/pdata/plog"
	"go.opentelemetry.io/collector/pdata/pmetric"
	"go.opentelemetry.io/collector/pdata/ptrace"
	"go.uber.org/zap"
)

// TestSignalSessions_RemoveSession tests removing sessions from SignalSessions
func TestSignalSessions_RemoveSession(t *testing.T) {
	t.Run("remove existing session", func(t *testing.T) {
		ss := &SignalSessions{
			sessions: map[uint32]*slim.BindingsSessionContext{
				1: nil, // Mock session using a nil pointer
			},
		}

		err := ss.RemoveSession(1)
		if err != nil {
			t.Errorf("expected no error, got %v", err)
		}
		if len(ss.sessions) != 0 {
			t.Errorf("expected 0 sessions, got %d", len(ss.sessions))
		}
	})

	t.Run("remove non-existing session", func(t *testing.T) {
		ss := &SignalSessions{
			sessions: map[uint32]*slim.BindingsSessionContext{},
		}

		err := ss.RemoveSession(1)
		if err == nil {
			t.Error("expected error, got nil")
		}
	})

	t.Run("remove session with nil sessions map", func(t *testing.T) {
		ss := &SignalSessions{}

		err := ss.RemoveSession(1)
		if err == nil {
			t.Error("expected error, got nil")
		}
	})
}

// TestSignalSessions_PublishToAll tests publishing data to all sessions
func TestSignalSessions_PublishToAll(t *testing.T) {
	logger := zap.NewNop()

	t.Run("publish to all sessions with empty map", func(t *testing.T) {
		ss := &SignalSessions{
			sessions: map[uint32]*slim.BindingsSessionContext{},
		}

		data := []byte("test data")
		closedSessions, err := ss.PublishToAll(data, logger, "test-signal")

		if err != nil {
			t.Errorf("expected no error, got %v", err)
		}
		if len(closedSessions) != 0 {
			t.Errorf("expected no closed sessions, got %d", len(closedSessions))
		}
	})
}

// TestExporterSessions_RemoveSessionForSignal tests removing sessions for different signal types
func TestExporterSessions_RemoveSessionForSignal(t *testing.T) {
	t.Run("remove metrics session", func(t *testing.T) {
		es := &ExporterSessions{
			metricsSessions: &SignalSessions{
				sessions: map[uint32]*slim.BindingsSessionContext{1: nil},
			},
		}

		err := es.RemoveSessionForSignal(common.SignalMetrics, 1)
		if err != nil {
			t.Errorf("expected no error, got %v", err)
		}
		if len(es.metricsSessions.sessions) != 0 {
			t.Errorf("expected empty sessions")
		}
	})

	t.Run("remove traces session", func(t *testing.T) {
		es := &ExporterSessions{
			tracesSessions: &SignalSessions{
				sessions: map[uint32]*slim.BindingsSessionContext{2: nil},
			},
		}

		err := es.RemoveSessionForSignal(common.SignalTraces, 2)
		if err != nil {
			t.Errorf("expected no error, got %v", err)
		}
		if len(es.tracesSessions.sessions) != 0 {
			t.Errorf("expected empty sessions")
		}
	})

	t.Run("remove logs session", func(t *testing.T) {
		es := &ExporterSessions{
			logsSessions: &SignalSessions{
				sessions: map[uint32]*slim.BindingsSessionContext{3: nil},
			},
		}

		err := es.RemoveSessionForSignal(common.SignalLogs, 3)
		if err != nil {
			t.Errorf("expected no error, got %v", err)
		}
		if len(es.logsSessions.sessions) != 0 {
			t.Errorf("expected empty sessions")
		}
	})

	t.Run("remove session with unknown signal type", func(t *testing.T) {
		es := &ExporterSessions{}

		err := es.RemoveSessionForSignal("unknown", 1)
		if err == nil {
			t.Error("expected error for unknown signal type")
		}
	})
}

// TestExporterSessions_RemoveAllSessionsForSignal tests removing all sessions for a signal type
func TestExporterSessions_RemoveAllSessionsForSignal(t *testing.T) {
	t.Run("remove all metrics sessions", func(t *testing.T) {
		// Note: We can't fully test this without mocking BindingsAdapter.DeleteSession
		// This is a basic structure test
		es := &ExporterSessions{
			metricsSessions: &SignalSessions{
				sessions: map[uint32]*slim.BindingsSessionContext{},
			},
		}

		// Should not panic
		es.RemoveAllSessionsForSignal(common.SignalMetrics)
	})

	t.Run("remove all sessions with unknown signal type", func(t *testing.T) {
		es := &ExporterSessions{
			metricsSessions: &SignalSessions{
				sessions: map[uint32]*slim.BindingsSessionContext{1: nil},
			},
		}

		// Should not panic or modify anything
		es.RemoveAllSessionsForSignal("unknown")
		if len(es.metricsSessions.sessions) != 1 {
			t.Errorf("expected 1 session to remain, got %d", len(es.metricsSessions.sessions))
		}
	})
}

// TestSlimExporter_PublishData tests the publishData method
func TestSlimExporter_PublishData(t *testing.T) {
	logger := zap.NewNop()

	// Reset global state before each test
	teardown := func() {
		mutex.Lock()
		state = nil
		mutex.Unlock()
	}

	t.Run("publish trace successfully", func(t *testing.T) {
		defer teardown()

		mutex.Lock()
		state = &ExporterSessions{
			tracesSessions: &SignalSessions{
				sessions: map[uint32]*slim.BindingsSessionContext{},
			},
		}
		mutex.Unlock()

		exporter := &slimExporter{
			config: &Config{
				SlimEndpoint: "test-endpoint",
			},
			logger:     logger,
			signalType: common.SignalTraces,
			sessions:   state,
		}

		data := []byte("test trace data")
		err := exporter.publishData(data, "traces", 5)

		if err != nil {
			t.Errorf("expected no error, got %v", err)
		}
	})

	t.Run("publish metrics successfully", func(t *testing.T) {
		defer teardown()

		mutex.Lock()
		state = &ExporterSessions{
			metricsSessions: &SignalSessions{
				sessions: map[uint32]*slim.BindingsSessionContext{},
			},
		}
		mutex.Unlock()

		exporter := &slimExporter{
			config: &Config{
				SlimEndpoint: "test-endpoint",
			},
			logger:     logger,
			signalType: common.SignalMetrics,
			sessions:   state,
		}

		data := []byte("test metric data")
		err := exporter.publishData(data, "metrics", 10)

		if err != nil {
			t.Errorf("expected no error, got %v", err)
		}
	})

	t.Run("publish logs successfully", func(t *testing.T) {
		defer teardown()

		mutex.Lock()
		state = &ExporterSessions{
			logsSessions: &SignalSessions{
				sessions: map[uint32]*slim.BindingsSessionContext{},
			},
		}
		mutex.Unlock()

		exporter := &slimExporter{
			config: &Config{
				SlimEndpoint: "test-endpoint",
			},
			logger:     logger,
			signalType: common.SignalLogs,
			sessions:   state,
		}

		data := []byte("test log data")
		err := exporter.publishData(data, "logs", 3)

		if err != nil {
			t.Errorf("expected no error, got %v", err)
		}
	})

	t.Run("publish with unknown signal type", func(t *testing.T) {
		defer teardown()

		exporter := &slimExporter{
			config: &Config{
				SlimEndpoint: "test-endpoint",
			},
			logger:     logger,
			signalType: "unknown",
		}

		data := []byte("test data")
		err := exporter.publishData(data, "unknown", 1)

		if err == nil {
			t.Error("expected error for unknown signal type")
		}
	})
}

// TestSlimExporter_PushTraces tests the pushTraces method
func TestSlimExporter_PushTraces(t *testing.T) {
	logger := zap.NewNop()

	t.Run("push traces with empty data", func(t *testing.T) {
		mutex.Lock()
		state = &ExporterSessions{
			tracesSessions: &SignalSessions{
				sessions: map[uint32]*slim.BindingsSessionContext{},
			},
		}
		mutex.Unlock()

		exporter := &slimExporter{
			config: &Config{
				SlimEndpoint: "test-endpoint",
			},
			logger:     logger,
			signalType: common.SignalTraces,
			sessions:   state,
		}

		td := ptrace.NewTraces()
		err := exporter.pushTraces(context.Background(), td)

		if err != nil {
			t.Errorf("expected no error, got %v", err)
		}

		// Cleanup
		mutex.Lock()
		state = nil
		mutex.Unlock()
	})
}

// TestSlimExporter_PushMetrics tests the pushMetrics method
func TestSlimExporter_PushMetrics(t *testing.T) {
	logger := zap.NewNop()

	t.Run("push metrics with empty data", func(t *testing.T) {
		mutex.Lock()
		state = &ExporterSessions{
			metricsSessions: &SignalSessions{
				sessions: map[uint32]*slim.BindingsSessionContext{},
			},
		}
		mutex.Unlock()

		exporter := &slimExporter{
			config: &Config{
				SlimEndpoint: "test-endpoint",
			},
			logger:     logger,
			signalType: common.SignalMetrics,
			sessions:   state,
		}

		md := pmetric.NewMetrics()
		err := exporter.pushMetrics(context.Background(), md)

		if err != nil {
			t.Errorf("expected no error, got %v", err)
		}

		// Cleanup
		mutex.Lock()
		state = nil
		mutex.Unlock()
	})
}

// TestSlimExporter_PushLogs tests the pushLogs method
func TestSlimExporter_PushLogs(t *testing.T) {
	logger := zap.NewNop()

	t.Run("push logs with empty data", func(t *testing.T) {
		mutex.Lock()
		state = &ExporterSessions{
			logsSessions: &SignalSessions{
				sessions: map[uint32]*slim.BindingsSessionContext{},
			},
		}
		mutex.Unlock()

		exporter := &slimExporter{
			config: &Config{
				SlimEndpoint: "test-endpoint",
			},
			logger:     logger,
			signalType: common.SignalLogs,
			sessions:   state,
		}

		ld := plog.NewLogs()
		err := exporter.pushLogs(context.Background(), ld)

		if err != nil {
			t.Errorf("expected no error, got %v", err)
		}

		// Cleanup
		mutex.Lock()
		state = nil
		mutex.Unlock()
	})
}

// TestConcurrentAccess tests concurrent access to SignalSessions
func TestConcurrentAccess(t *testing.T) {
	t.Run("concurrent map initialization", func(t *testing.T) {
		ss := &SignalSessions{}
		logger := zap.NewNop()
		var wg sync.WaitGroup

		// Initialize the map safely
		ss.mutex.Lock()
		if ss.sessions == nil {
			ss.sessions = make(map[uint32]*slim.BindingsSessionContext)
		}
		ss.mutex.Unlock()

		// Test concurrent publish operations
		for i := 0; i < 10; i++ {
			wg.Add(1)
			go func() {
				defer wg.Done()
				data := []byte("test data")
				_, _ = ss.PublishToAll(data, logger, "test")
			}()
		}

		wg.Wait()

		// Verify no panics occurred
		if ss.sessions == nil {
			t.Error("expected sessions map to be initialized")
		}
	})
}
