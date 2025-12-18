package slimexporter

import (
	"testing"

	"go.opentelemetry.io/collector/component"
)

func TestNewFactory(t *testing.T) {
	factory := NewFactory()

	if factory == nil {
		t.Fatal("NewFactory() returned nil")
	}

	if factory.Type() != component.MustNewType(TypeStr) {
		t.Errorf("Type() = %v, want %v", factory.Type(), TypeStr)
	}
}

func TestCreateDefaultConfig(t *testing.T) {
	factory := NewFactory()
	cfg := factory.CreateDefaultConfig()

	if cfg == nil {
		t.Fatal("CreateDefaultConfig() returned nil")
	}

	slimCfg, ok := cfg.(*Config)
	if !ok {
		t.Fatalf("CreateDefaultConfig() returned wrong type: %T", cfg)
	}

	if slimCfg.SlimEndpoint != "http://127.0.0.1:46357" {
		t.Errorf("SlimEndpoint = %v, want http://127.0.0.1:46357", slimCfg.SlimEndpoint)
	}

	if slimCfg.LocalName != "agntcy/otel/exporter" {
		t.Errorf("LocalName = %v, want agntcy/otel/exporter", slimCfg.LocalName)
	}
}

func TestFactoryType(t *testing.T) {
	factory := NewFactory()
	expectedType := component.MustNewType("slim")

	if factory.Type() != expectedType {
		t.Errorf("Type() = %v, want %v", factory.Type(), expectedType)
	}
}
