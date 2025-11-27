FROM alpine:3.19 AS certs
RUN apk --update add ca-certificates

# Build the Rust SLIM library
FROM rust:1.91-bookworm AS rust-build
WORKDIR /rust-build

# Copy the entire data-plane directory to build the Rust library
COPY ./data-plane ./data-plane
WORKDIR /rust-build/data-plane

# Build the SLIM service library
RUN cargo build --release -p agntcy-slim-service

# Build the OpenTelemetry Collector
FROM golang:1.25.4 AS build-stage
WORKDIR /build

# Copy the Rust library from the previous stage
COPY --from=rust-build /rust-build/data-plane/target/release/libslim_service.* /usr/local/lib/

# Copy files and create a builder config with absolute paths for Docker
COPY ./slimexporter ./slimexporter
COPY ./generated ./generated
COPY ./common ./common
COPY ./builder-config.yaml builder-config.yaml

# Install the OpenTelemetry Collector Builder
RUN --mount=type=cache,target=/root/.cache/go-build GO111MODULE=on go install go.opentelemetry.io/collector/cmd/builder@v0.140.0

# Create a Docker-specific builder config with absolute paths
RUN sed -i 's|../../../generated|/build/generated|g' builder-config.yaml && \
    sed -i 's|../../common|/build/common|g' builder-config.yaml && \
    sed -i 's|../slimexporter|/build/slimexporter|g' builder-config.yaml

# Generate the collector sources without compiling
RUN --mount=type=cache,target=/root/.cache/go-build builder --config builder-config.yaml --skip-compilation

# Compile with CGO enabled (required for SLIM bindings)
WORKDIR /build/slim-otelcol
RUN --mount=type=cache,target=/root/.cache/go-build \
    CGO_ENABLED=1 \
    CGO_LDFLAGS="-L/usr/local/lib -lslim_service" \
    LD_LIBRARY_PATH="/usr/local/lib:$LD_LIBRARY_PATH" \
    go build -trimpath -o otelcol-slim -ldflags="-s -w"

FROM gcr.io/distroless/cc-debian12:latest

ARG USER_UID=10001

# Copy the Rust library
COPY --from=rust-build /rust-build/data-plane/target/release/libslim_service.so /lib/

COPY ./collector-config.yaml /otelcol/collector-config.yaml
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --chmod=755 --from=build-stage /build/slim-otelcol/otelcol-slim /otelcol/otelcol-slim

# Create home directory with .slim subdirectory for SLIM storage
COPY --from=build-stage --chown=${USER_UID}:${USER_UID} --chmod=755 /tmp /home/otel/.slim

USER ${USER_UID}

# Set library path and home directory
ENV LD_LIBRARY_PATH=/lib
ENV HOME=/home/otel

ENTRYPOINT ["/otelcol/otelcol-slim"]
CMD ["--config", "/otelcol/collector-config.yaml"]

EXPOSE 4317 4318

