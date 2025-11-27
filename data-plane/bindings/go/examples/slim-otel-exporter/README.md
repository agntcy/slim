# SLIM OpenTelemetry Collector Exporter

## Overview

This directory contains an OpenTelemetry Collector with a custom SLIM exporter that enables secure, privacy-preserving telemetry data transmission using the SLIM protocol. The exporter can operate in two modes:
- **Moderator mode**: Creates channels and invites participants
- **Participant mode**: Joins existing channels created by a moderator

## Project Structure

- `slimexporter/` - Custom SLIM exporter implementation
  - `factory.go` - Factory for creating exporter instances
  - `exporter.go` - Main exporter logic for traces, metrics, and logs
  - `config.go` - Configuration structure
  - `go.mod` - Go module dependencies
- `builder-config.yaml` - OCB (OpenTelemetry Collector Builder) configuration
- `collector-config.yaml` - Runtime configuration with SLIM exporter settings
- `build.sh` - Build script for the collector
- `docker-build.sh` - Docker image build script

## Building the Collector

### Local Build

Before start you need to install the Opentelemetry Collector Bulder (OCB)
To do so follow the Step 1 on the official doc here https://opentelemetry.io/docs/collector/custom-collector/#step-1---install-the-builder

Use the provided build script to compile the collector with CGO support (required for SLIM Go bindings):

```bash
./build.sh
```

This will:
1. Generate the collector sources using OCB
2. Compile with CGO enabled
3. Output the binary to `slim-otelcol/slim-otelcol`

### Docker Build

To create a Docker image:

```bash
./docker-build.sh
```

This builds a Docker image named `slim-otel-collector` with all necessary dependencies.

## Configuration

The collector is configured via `collector-config.yaml`. Key SLIM exporter settings:

```yaml
exporters:
  slim:
    endpoint: "http://127.0.0.1:46357"        # SLIM node address
    local-name: "agntcy/otel/exporter"        # Exporter identity
    channel-name: "agntcy/otel/telemetry"     # Base channel name (moderator mode)
    participants-list:                        # Participants to invite (moderator mode)
      - "agntcy/otel/receiver-app"
    mls-enabled: false                        # Enable/disable MLS encryption
    shared-secret: "your-shared-secret"       # Shared secret for MLS and identity
```

**Note**: When running as moderator, three channels are created automatically:
- `agntcy/otel/telemetry-metrics`
- `agntcy/otel/telemetry-traces`
- `agntcy/otel/telemetry-logs`

## Running the Collector

### Local Execution

After building, run the collector:

```bash
cd slim-otelcol
./slim-otelcol --config ../collector-config.yaml
```

### Docker Execution

```bash
docker run -p 4317:4317 -p 4318:4318 slim-otel-collector
```

The collector exposes:
- **gRPC**: `0.0.0.0:4317` for OTLP/gRPC
- **HTTP**: `0.0.0.0:4318` for OTLP/HTTP

## Testing the Exporter

To test the SLIM exporter, you need to set up a complete environment:

### 1. Start the SLIM Server

From the project root, run the SLIM example server:

```bash
task example:server
```

This starts the SLIM node that the exporter will connect to.

### 2. Run the Collector

Start the collector as described above (either locally or via Docker).

### 3. Run a Receiver Application

Depending on how you configured the exporter, run the appropriate receiver:

**For Moderator Mode** (when `participants-list` is specified):
```bash
task example:otel-receiver:participant
```

**For Participant Mode** (when joining an existing channel):
```bash
task example:otel-receiver:moderator
```

## Deployment Modes

### Moderator Mode

Configure the exporter to create channels and invite participants:

```yaml
exporters:
  slim:
    channel-name: "agntcy/otel/telemetry"
    participants-list:
      - "agntcy/otel/receiver-app"
```

Use with: `task example:otel-receiver:participant`

### Participant Mode

Remove `participants-list` to join existing channels:

```yaml
exporters:
  slim:
    # No participants-list needed
```

Use with: `task example:otel-receiver:moderator`
