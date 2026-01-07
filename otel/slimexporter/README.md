# SLIM Exporter

The SLIM exporter sends OpenTelemetry traces, metrics, and logs to a [SLIM](https://github.com/agntcy/slim) (Secure Low-Latency Interactive Messaging) channel. SLIM facilitates secure, low-latency communication between ai-agents and applications using various communication patterns such as point-to-point or groups. This exporter enables secure distribution of observability data using SLIM's end-to-end encryption with MLS (Message Layer Security) and flexible channel-based routing.

## Configuration settings

The following settings are required:

- `endpoint` (default = `http://127.0.0.1:46357`): The address of the SLIM node to connect to.
- `local-name` (default = `agntcy/otel/exporter`): The local name identifier for this exporter in the SLIM form `org/namespace/service`.
- `shared-secret` (no default): The shared secret used for MLS and identity provider authentication.

The following settings can be optionally configured:

- `sessions` (default = `[]`): A list of session/channel configurations to create. When the list is empty, the exporter operates in passive mode, only listening for invitations from other participants. When sessions are configured, the exporter actively creates those sessions and invites participants, while also continuing to listen for incoming invitations from other participants.

### Session Configuration

Each session in the `sessions` array supports the following configuration:

- `channel-name` (required): The base name of the SLIM channel in the form `org/namespace/service`. The actual channel names will be suffixed with the signal type (e.g., `channel-name-traces`, `channel-name-metrics`, `channel-name-logs`).
- `signals` (required): An array of signal types to export on this channel. Valid values are `traces`, `metrics`, and `logs`.
- `participants` (required): An array of participant identifiers to invite to the channel.
- `mls-enabled` (default = `false`): Flag to enable or disable MLS (Message Layer Security) encryption for this session.

### Example configuration

Example configuration with multiple sessions:

```yaml
exporters:
  slim:
    endpoint: "http://127.0.0.1:46357"
    local-name: "agntcy/otel/exporter"
    shared-secret: "a-very-long-shared-secret-0123456789-abcdefg"
    sessions: 
      - channel-name: "agntcy/otel/telemetry-1"
        signals: 
          - metrics
          - logs
        participants:
          - "agntcy/otel/receiver-app"
        mls-enabled: false
      
      - channel-name: "agntcy/otel/telemetry-2"
        signals: 
          - metrics
        participants:
          - "agntcy/otel/receiver-app-2"
          - "agntcy/otel/receiver-app-3"
        mls-enabled: true
```

Example configuration listening for invitations:

```yaml
exporters:
  slim:
    endpoint: "http://127.0.0.1:46357"
    local-name: "agntcy/otel/passive-exporter"
    shared-secret: "a-very-long-shared-secret-0123456789-abcdefg"
    # No sessions defined - will listen for invitations
```

Complete pipeline configuration:

```yaml
receivers:
  otlp:
    protocols:
      grpc:
        endpoint: 0.0.0.0:4317
      http:
        endpoint: 0.0.0.0:4318

processors:
  batch:
    timeout: 1s
    send_batch_size: 1024

exporters:
  slim:
    endpoint: "http://127.0.0.1:46357"
    local-name: "agntcy/otel/exporter"
    shared-secret: "a-very-long-shared-secret-0123456789-abcdefg"
    sessions: 
      - channel-name: "agntcy/otel/telemetry"
        signals: 
          - traces
          - metrics
          - logs
        participants:
          - "agntcy/otel/receiver-app"
        mls-enabled: false

service:
  pipelines:
    traces:
      receivers: [otlp]
      processors: [batch]
      exporters: [slim]
    
    metrics:
      receivers: [otlp]
      processors: [batch]
      exporters: [slim]
    
    logs:
      receivers: [otlp]
      processors: [batch]
      exporters: [slim]
```

## How it works

The SLIM exporter:

1. **Connects** to a SLIM node using the configured endpoint and authenticates using the shared secret.
2. **Creates sessions** based on the `sessions` configuration, automatically suffixing channel names with the signal type (e.g., `-traces`, `-metrics`, `-logs`).
3. **Invites participants** to the created channels if configured.
4. **Publishes** OpenTelemetry data (serialized as protobuf) to the appropriate SLIM channels based on signal type.
5. **Listens for invitations** from other participants to join additional channels.

The exporter can operate in multiple modes simultaneously: it can create and manage its own sessions while also accepting invitations to join channels created by other participants.

### Signal Routing

Each signal type (traces, metrics, logs) is routed to separate channels:
- Traces → `channel-name-traces`
- Metrics → `channel-name-metrics`
- Logs → `channel-name-logs`

This allows for fine-grained control over which participants receive which types of telemetry data.

### Security

The SLIM exporter supports end-to-end encryption through MLS (Message Layer Security - RFC 9420) when `mls-enabled` is set to `true` for a session.

## Additional Information

- [SLIM Project](https://github.com/agntcy/slim)
- [SLIM Documentation](https://docs.agntcy.org/messaging/slim-core/)
- [MLS RFC 9420](https://datatracker.ietf.org/doc/rfc9420/)
