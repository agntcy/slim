# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

# Build stage
ARG BUILDPLATFORM
FROM --platform=$BUILDPLATFORM golang:1.25.5-bookworm AS builder

# Install required packages
RUN apt-get update && apt-get install -y \
    curl \
    gcc \
    libc6-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Download OCB for the build platform
ARG OCB_VERSION=0.142.0
RUN case $(uname -m) in \
        x86_64) ARCH=amd64 ;; \
        aarch64) ARCH=arm64 ;; \
        *) echo "Unsupported architecture" && exit 1 ;; \
    esac && \
    OS=$(uname -s | tr '[:upper:]' '[:lower:]') && \
    echo "Downloading OCB ${OCB_VERSION} for ${OS}/${ARCH}..." && \
    curl -L -o ocb "https://github.com/open-telemetry/opentelemetry-collector-releases/releases/download/cmd%2Fbuilder%2Fv${OCB_VERSION}/ocb_${OCB_VERSION}_${OS}_${ARCH}" && \
    chmod +x ocb

# Copy builder config and source code
COPY builder-config.yaml .
COPY go.mod ./
COPY common.go .
COPY slimexporter/ ./slimexporter/

# Generate collector sources
RUN ./ocb --config builder-config.yaml --skip-compilation

# Build the collector with CGO enabled
WORKDIR /build/slim-otelcol
RUN go mod download && \
    go run github.com/agntcy/slim-bindings-go/cmd/slim-bindings-setup && \
    CGO_ENABLED=1 CGO_LDFLAGS="-L/root/.cache/slim-bindings" go build -trimpath -o slim-otelcol -ldflags="-s -w"

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    wget \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the collector binary
COPY --from=builder /build/slim-otelcol/slim-otelcol .

# Expose standard OTLP ports
EXPOSE 4317 4318

ENTRYPOINT ["/app/slim-otelcol"]
CMD ["--config=/etc/otel/config.yaml"]
