# OpenTelemetry HTTP Receiver with SLIM Integration

A Python application that receives OpenTelemetry metrics and traces via HTTP POST requests on port 4318 (the standard OpenTelemetry HTTP port) and forwards them to a SLIM group channel for secure, distributed processing.

## Features

- **HTTP Server**: Listens on port 4318 for OpenTelemetry data (metrics and traces)
- **SLIM Integration**: Forwards received OTel data to a SLIM group session
- **Dual Modes**: 
  - **Moderator Mode**: Creates a SLIM session and invites participants
  - **Participant Mode**: Waits for invitation to join an existing session

## Architecture

The application runs in two scenarios:

### Scenario 1: OTel Receiver as Moderator
1. SLIM server runs (`python:example:server`)
2. SLIM participant waits for invitation (`python:example:otel:slim-participant`)
3. OTel receiver creates session and invites participant (`python:example:otel:otel-receiver-moderator`)
4. HTTP requests to OTel receiver are forwarded to SLIM channel

### Scenario 2: OTel Receiver as Participant  
1. SLIM server runs (`python:example:server`)
2. OTel receiver waits for invitation (`python:example:otel:otel-receiver-participant`)
3. SLIM moderator creates session and invites OTel receiver (`python:example:otel:slim-moderator`)
4. HTTP requests to OTel receiver are forwarded to SLIM channel

## Usage

### Prerequisites

First, always start the SLIM server:

```bash
task python:example:server
```

### Scenario 1: OTel Receiver Creates the Session

**Terminal 1** - Start SLIM participant (waits for invitation):
```bash
task python:example:otel:slim-participant
```

**Terminal 2** - Start OTel receiver as moderator (creates session and invites participant):
```bash
task python:example:otel:otel-receiver-moderator
```

**Terminal 3** - Send OTel data:
```bash
curl -X POST http://localhost:4318/v1/traces \
  -H "Content-Type: application/json" \
  -d '{"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"test-service"}}]}}]}'
```

### Scenario 2: OTel Receiver Joins Existing Session

**Terminal 1** - Start OTel receiver as participant (waits for invitation):
```bash
task python:example:otel:otel-receiver-participant
```

**Terminal 2** - Start SLIM moderator (creates session and invites OTel receiver):
```bash
task python:example:otel:slim-moderator
```

**Terminal 3** - Send OTel data:
```bash
curl -X POST http://localhost:4318/v1/traces \
  -H "Content-Type: application/json" \
  -d '{"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"test-service"}}]}}]}'
```

## Testing

### Send Traces
```bash
curl -X POST http://localhost:4318/v1/traces \
  -H "Content-Type: application/json" \
  -d '{"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"test-service"}}]}}]}'
```

### Send Metrics
```bash
curl -X POST http://localhost:4318/v1/metrics \
  -H "Content-Type: application/json" \
  -d '{"resourceMetrics":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"test-service"}}]}}]}'
```

## Configuration

### Task Commands Reference

- `python:example:server` - Start the SLIM server (required first)
- `python:example:otel:otel-receiver-moderator` - OTel receiver creates session
- `python:example:otel:otel-receiver-participant` - OTel receiver waits for invitation
- `python:example:otel:slim-moderator` - SLIM moderator invites OTel receiver
- `python:example:otel:slim-participant` - SLIM participant waits for invitation
