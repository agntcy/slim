# Deployment: Docker

Running SLIM with Docker is a good fit for containerised single-host deployments where you want agents and the SLIM node to run together as a self-contained stack. It is also useful for CI pipelines, portable demos, or any environment where you want containerised components without Kubernetes.

## Container Images

SLIM images are published to the GitHub Container Registry:

| Image | Tag | Description |
|-------|-----|-------------|
| `ghcr.io/agntcy/slim` | `<version>` | SLIM data plane (distroless, production) |
| `ghcr.io/agntcy/slim` | `<version>-debug` | SLIM data plane (Debian slim, includes shell) |
| `ghcr.io/agntcy/slim/control-plane` | `<version>` | SLIM Controller (distroless) |
| `ghcr.io/agntcy/slim/control-plane` | `<version>-debug` | SLIM Controller (Debian slim) |

Both `linux/amd64` and `linux/arm64` are supported.

## Single Node

Create a minimal config file:

```yaml
# config.yaml
tracing:
  log_level: info
  display_thread_names: true
  display_thread_ids: true

runtime:
  n_cores: 0
  thread_name: "slim-data-plane"
  drain_timeout: 10s

services:
  slim/0:
    node_id: slim-docker
    dataplane:
      servers:
        - endpoint: "0.0.0.0:46357"
          tls:
            insecure: true
      clients: []
```

Run the SLIM node:

```bash
docker run -it \
    -v ./config.yaml:/config.yaml \
    -p 46357:46357 \
    ghcr.io/agntcy/slim:1.4.0 /slim --config /config.yaml
```

SDK applications on the host connect to `http://127.0.0.1:46357`.

## Docker Compose: Data Plane with Client Apps

The `crates/examples/` directory includes a Docker Compose file that starts a SLIM node alongside mock client applications for quick end-to-end testing.

```yaml
# docker-compose.yml
services:
  slim-server:
    image: ghcr.io/agntcy/slim:1.4.0
    entrypoint: ["/slim"]
    command: ["--config", "/config/server-config.yaml"]
    ports:
      - "50001:50001"
    volumes:
      - ./server-config.yaml:/config/server-config.yaml
    networks:
      - slim-network

  mock-app-server:
    image: ghcr.io/agntcy/slim/examples:1.4.0
    depends_on:
      - slim-server
    networks:
      - slim-network

  mock-app-client:
    image: ghcr.io/agntcy/slim/examples:1.4.0
    depends_on:
      - slim-server
      - mock-app-server
    networks:
      - slim-network

networks:
  slim-network:
    driver: bridge
```

Start everything:

```bash
docker compose up
```

## Docker Compose: Adding the Telemetry Stack

To add distributed tracing and metrics, include the telemetry stack alongside your SLIM node. This starts Jaeger, an OpenTelemetry Collector, and Prometheus.

```yaml
# docker-compose.telemetry.yml
services:
  jaeger:
    image: jaegertracing/all-in-one:latest
    ports:
      - "16686:16686"   # Jaeger UI
      - "14317:14317"   # OTLP gRPC collector
    environment:
      - COLLECTOR_OTLP_ENABLED=true
      - COLLECTOR_OTLP_GRPC_HOST_PORT=:14317

  otel-collector:
    image: otel/opentelemetry-collector-contrib:latest
    command: ["--config=/etc/otel-collector-config.yml"]
    ports:
      - "4317:4317"    # OTLP gRPC receiver
      - "9091:9091"    # Prometheus exporter
    depends_on:
      - jaeger

  prometheus:
    image: prom/prometheus:latest
    ports:
      - "9090:9090"    # Prometheus UI
    depends_on:
      - otel-collector
```

Start with both compose files:

```bash
docker compose -f docker-compose.yml -f docker-compose.telemetry.yml up
```

Then configure your SLIM node to export telemetry to `http://otel-collector:4317`. The Jaeger UI is available at `http://localhost:16686` and Prometheus at `http://localhost:9090`.

## Running the SLIM Controller

To run the Controller alongside the data plane:

```bash
docker run -it \
    -v ./controller-config.yaml:/config.yaml \
    -v ./db:/db \
    -p 50051:50051 -p 50052:50052 \
    ghcr.io/agntcy/slim/control-plane:1.4.0 \
    -config /config.yaml
```

See the [Controller Configuration Reference](../components/controller/config.md) for the controller config format.

## Next Steps

- [Kubernetes: Single Cluster](./kubernetes.md) — Deploy SLIM on Kubernetes
- [Data Plane Configuration Reference](../components/data-plane/config.md) — Full config options for the data plane
- [OpenTelemetry over SLIM](../integrations/otel.md) — Exporting telemetry through the SLIM network
