package slimexporter

import (
	"context"
	"fmt"

	"go.opentelemetry.io/collector/component"
	"go.opentelemetry.io/collector/exporter"
	"go.opentelemetry.io/collector/exporter/exporterhelper"
)

const (
	// TypeStr is the type of the exporter
	TypeStr = "slim"

	// The stability level of the exporter
	stability = component.StabilityLevelDevelopment
)

// NewFactory creates a factory for the Slim exporter
func NewFactory() exporter.Factory {
	return exporter.NewFactory(
		component.MustNewType(TypeStr),
		createDefaultConfig,
		exporter.WithTraces(createTracesExporter, stability),
		exporter.WithMetrics(createMetricsExporter, stability),
		exporter.WithLogs(createLogsExporter, stability),
	)
}

// createDefaultConfig creates the default configuration for the exporter
func createDefaultConfig() component.Config {
	return &Config{
		Endpoint: "http://localhost:8080",
	}
}

// createTracesExporter creates a trace exporter based on the config
func createTracesExporter(
	ctx context.Context,
	set exporter.Settings,
	cfg component.Config,
) (exporter.Traces, error) {
	exporterConfig := cfg.(*Config)

	if err := exporterConfig.Validate(); err != nil {
		return nil, fmt.Errorf("invalid config: %w", err)
	}

	exp := newSlimExporter(exporterConfig, set.Logger)

	return exporterhelper.NewTraces(
		ctx,
		set,
		cfg,
		exp.pushTraces,
		exporterhelper.WithStart(exp.start),
		exporterhelper.WithShutdown(exp.shutdown),
		exporterhelper.WithCapabilities(exp.Capabilities()),
	)
}

// createMetricsExporter creates a metrics exporter based on the config
func createMetricsExporter(
	ctx context.Context,
	set exporter.Settings,
	cfg component.Config,
) (exporter.Metrics, error) {
	exporterConfig := cfg.(*Config)

	if err := exporterConfig.Validate(); err != nil {
		return nil, fmt.Errorf("invalid config: %w", err)
	}

	exp := newSlimExporter(exporterConfig, set.Logger)

	return exporterhelper.NewMetrics(
		ctx,
		set,
		cfg,
		exp.pushMetrics,
		exporterhelper.WithStart(exp.start),
		exporterhelper.WithShutdown(exp.shutdown),
		exporterhelper.WithCapabilities(exp.Capabilities()),
	)
}

// createLogsExporter creates a logs exporter based on the config
func createLogsExporter(
	ctx context.Context,
	set exporter.Settings,
	cfg component.Config,
) (exporter.Logs, error) {
	exporterConfig := cfg.(*Config)

	if err := exporterConfig.Validate(); err != nil {
		return nil, fmt.Errorf("invalid config: %w", err)
	}

	exp := newSlimExporter(exporterConfig, set.Logger)

	return exporterhelper.NewLogs(
		ctx,
		set,
		cfg,
		exp.pushLogs,
		exporterhelper.WithStart(exp.start),
		exporterhelper.WithShutdown(exp.shutdown),
		exporterhelper.WithCapabilities(exp.Capabilities()),
	)
}
