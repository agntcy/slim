package slimexporter

import (
	"context"

	"go.opentelemetry.io/collector/component"
	"go.opentelemetry.io/collector/consumer"
	"go.opentelemetry.io/collector/pdata/plog"
	"go.opentelemetry.io/collector/pdata/pmetric"
	"go.opentelemetry.io/collector/pdata/ptrace"
	"go.uber.org/zap"
)

// slimExporter implements the exporter for traces, metrics, and logs
type slimExporter struct {
	config *Config
	logger *zap.Logger

	// Add any custom fields needed for your exporter
	// For example: httpClient, connection pool, etc.
}

// newSlimExporter creates a new instance of the slim exporter
func newSlimExporter(cfg *Config, logger *zap.Logger) *slimExporter {
	return &slimExporter{
		config: cfg,
		logger: logger,
	}
}

// start is invoked during service startup
func (e *slimExporter) start(ctx context.Context, host component.Host) error {
	e.logger.Info("Starting Slim exporter", zap.String("endpoint", e.config.Endpoint))

	// Initialize connections, clients, or any resources needed
	// For example: establish HTTP client, database connections, etc.

	return nil
}

// shutdown is invoked during service shutdown
func (e *slimExporter) shutdown(ctx context.Context) error {
	e.logger.Info("Shutting down Slim exporter")

	// Clean up resources, close connections, etc.

	return nil
}

// pushTraces exports trace data
func (e *slimExporter) pushTraces(ctx context.Context, td ptrace.Traces) error {
	// Implement your trace export logic here
	spanCount := td.SpanCount()
	e.logger.Info("Exporting traces",
		zap.Int("span_count", spanCount),
		zap.String("endpoint", e.config.Endpoint))

	// Example: Convert and send traces to your backend
	// This is where you'd implement the actual export logic

	return nil
}

// pushMetrics exports metrics data
func (e *slimExporter) pushMetrics(ctx context.Context, md pmetric.Metrics) error {
	// Implement your metrics export logic here
	dataPointCount := md.DataPointCount()
	e.logger.Info("Exporting metrics",
		zap.Int("data_point_count", dataPointCount),
		zap.String("endpoint", e.config.Endpoint))

	// Example: Convert and send metrics to your backend

	return nil
}

// pushLogs exports logs data
func (e *slimExporter) pushLogs(ctx context.Context, ld plog.Logs) error {
	// Implement your logs export logic here
	logRecordCount := ld.LogRecordCount()
	e.logger.Info("Exporting logs",
		zap.Int("log_record_count", logRecordCount),
		zap.String("endpoint", e.config.Endpoint))

	// Example: Convert and send logs to your backend

	return nil
}

// Capabilities returns the consumer capabilities of the exporter
func (e *slimExporter) Capabilities() consumer.Capabilities {
	return consumer.Capabilities{MutatesData: false}
}
