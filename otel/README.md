# SLIM OpenTelemetry Collector

This directory contains the SLIM OpenTelemetry Collector, a custom distribution of the OpenTelemetry Collector that includes the SLIM exporter for secure communication channels.

## Building the Collector

To build the SLIM OpenTelemetry Collector:

```bash
task collector:build
```

This command will:
1. Download OCB if not already present
2. Generate collector sources based on `builder-config.yaml`
3. Output the binary to `./slim-otelcol/slim-otelcol`

## Running the Collector

### Run Locally

To run the SLIM OpenTelemetry Collector with the default configuration:

```bash
task collector:run
```

The collector will use the configuration defined in `collector-config.yaml`.

### Run with Docker

Build the Docker image:

```bash
task docker:build
```

Run the collector in a Docker container:

```bash
task docker:run
```

This will start the collector with:
- OTLP gRPC receiver on port 4317
- OTLP HTTP receiver on port 4318
- Configuration mounted from `collector-config.yaml`

## Testing

### Run Exporter Tests

To run unit tests for the SLIM exporter:

```bash
task collector:test
```

### End-to-End Testing

#### 1. Start a SLIM Node

First, start a SLIM node for testing:

```bash
task test:slim:run
```

This runs a SLIM data plane node with the base server configuration.

#### 2. Start the Collector

In a separate terminal, start the SLIM OpenTelemetry Collector:

```bash
task collector:run
```

#### 3. Run Test Applications

The application can be run in two modes:

**Participant Mode**: The collector adds the participant to the channel specified in `collector-config.yaml`:

```bash
task test:app:participant
```

The collector must have the participant configured in its session list in `collector-config.yaml`.

**Initiator Mode**: The test app invites the collector to join a channel:

```bash
task test:app:initiator
```

When using initiator mode, the collector's session list in `collector-config.yaml` can be set to `[]` since the app will invite the collector dynamically.

In both cases, telemetry data will flow through the SLIM collector over secure SLIM channels and will be logged by the test application.

## Configuration

The SLIM collector can be configured using the `collector-config.yaml` file. Check the README.md in `./slimexporter` for a complete description of the configuration options.
