# Custom OpenTelemetry Exporter - Quick Start

## Overview
This directory contains a custom OpenTelemetry Collector exporter called "Slim Exporter".

## Structure
- `slimexporter/` - Custom exporter implementation
  - `factory.go` - Factory for creating exporter instances
  - `exporter.go` - Main exporter logic for traces, metrics, and logs
  - `config.go` - Configuration structure
  - `go.mod` - Go module dependencies
- `builder-config.yaml` - OCB (OpenTelemetry Collector Builder) configuration
- `collector-config.yaml` - Sample collector runtime configuration

## Build the Custom Collector

1. **Install dependencies:**
   ```bash
   cd slimexporter
   go mod tidy
   cd ..
   ```

2. **Build the collector using OCB:**
   ```bash
   # If you don't have ocb installed:
   # go install go.opentelemetry.io/collector/cmd/builder@v0.140.0
   
   ocb --config builder-config.yaml
   ```

## Run the Collector

```bash
./slim-otelcol/slim-otelcol --config collector-config.yaml
```

## Customize Your Exporter

### Add Custom Configuration
Edit `slimexporter/config.go` to add new configuration fields:

```go
type Config struct {
    Endpoint string `mapstructure:"endpoint"`
    APIKey   string `mapstructure:"api_key"`
    Timeout  time.Duration `mapstructure:"timeout"`
}
```

### Implement Export Logic
Edit the push methods in `slimexporter/exporter.go`:

- `pushTraces()` - Handle trace data export
- `pushMetrics()` - Handle metrics data export  
- `pushLogs()` - Handle logs data export

### Example: HTTP Export
```go
func (e *slimExporter) pushTraces(ctx context.Context, td ptrace.Traces) error {
    // Convert to JSON
    marshaler := &ptrace.JSONMarshaler{}
    data, err := marshaler.MarshalTraces(td)
    if err != nil {
        return err
    }
    
    // Send HTTP request
    req, err := http.NewRequestWithContext(ctx, "POST", e.config.Endpoint, bytes.NewReader(data))
    if err != nil {
        return err
    }
    req.Header.Set("Content-Type", "application/json")
    
    resp, err := http.DefaultClient.Do(req)
    if err != nil {
        return err
    }
    defer resp.Body.Close()
    
    return nil
}
```

## Test Your Exporter

Send sample telemetry data using the OTLP protocol:

```bash
# Install telemetrygen for testing
go install github.com/open-telemetry/opentelemetry-collector-contrib/cmd/telemetrygen@latest

# Generate traces
telemetrygen traces --otlp-insecure --traces 10

# Generate metrics
telemetrygen metrics --otlp-insecure --metrics 10

# Generate logs
telemetrygen logs --otlp-insecure --logs 10
```

## Next Steps

1. Implement your actual export logic in the `push*()` methods
2. Add authentication/authorization as needed
3. Add retry logic and error handling
4. Add metrics/observability for the exporter itself
5. Write unit tests for your exporter
6. Consider adding batching, compression, or other optimizations
