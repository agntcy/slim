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

# Copy pre-built Rust bindings (copied locally by build task)
# These must be built separately for Linux before running docker build
COPY data-plane/bindings/go/generated/ ./data-plane/bindings/go/generated/

# Verify the library file exists
RUN ls -la ./data-plane/bindings/go/generated/slim_bindings/ && \
    echo "Checking for library files..." && \
    find ./data-plane/bindings/go/generated/slim_bindings/ -name "*.so" -o -name "*.dylib"

# Adjust replace paths in builder-config for Docker context
# Note: paths are relative to the generated slim-otelcol/ directory
RUN sed -e 's|=> \.\./\.\./data-plane|=> ../data-plane|g' \
        -e 's|=> \.\.$|=> ..|g' \
        builder-config.yaml > builder-config-docker.yaml && \
    cat builder-config-docker.yaml

# Generate collector sources
RUN ./ocb --config builder-config-docker.yaml --skip-compilation

# Build the collector with CGO enabled
WORKDIR /build/slim-otelcol
ENV CGO_LDFLAGS="-L/build/data-plane/bindings/go/generated/slim_bindings -lslim_bindings"
ENV CGO_CFLAGS="-I/build/data-plane/bindings/go/generated/slim_bindings"
RUN CGO_ENABLED=1 go build -trimpath -o slim-otelcol -ldflags="-s -w"

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

# Copy the native library
COPY --from=builder /build/data-plane/bindings/go/generated/slim_bindings/libslim_bindings.* /usr/local/lib/

# Set library path
ENV LD_LIBRARY_PATH=/usr/local/lib

# Copy default config
COPY collector-config.yaml /etc/otel/config.yaml

# Expose standard OTLP ports
EXPOSE 4317 4318

ENTRYPOINT ["/app/slim-otelcol"]
CMD ["--config=/etc/otel/config.yaml"]
