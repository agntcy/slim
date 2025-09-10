# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

# Build container
FROM --platform=${BUILDPLATFORM} rust:1.87-slim-bookworm AS rust

SHELL ["/bin/bash", "-c"]

ARG TARGETARCH

RUN DEBIAN_FRONTEND=noninteractive \
    apt-get update && \
    apt-get install --no-install-recommends -y \
        curl \
        file \
        make \
        unzip \
        git \
        lsb-release \
        pkg-config && \
    curl -OL https://apt.llvm.org/llvm.sh && \
    chmod +x llvm.sh && \
    ./llvm.sh 19

# Install taskfile
RUN sh -c "$(curl --location https://taskfile.dev/install.sh)" -- -d -b /usr/local/bin

# Copy source code
COPY . /app
WORKDIR /app/data-plane

RUN <<EOF
case ${TARGETARCH} in
    "amd64")
        PACKAGES="gcc-x86-64-linux-gnu g++-x86-64-linux-gnu"
        RUSTARCH="x86_64-unknown-linux-gnu"
        ;;
    "arm64")
        PACKAGES="gcc-aarch64-linux-gnu g++-aarch64-linux-gnu"
        RUSTARCH="aarch64-unknown-linux-gnu"
        ;;
    *)
        echo "Unsupported platform: ${TARGETPLATFORM}"
        exit 1
        ;;
esac

apt-get update && apt-get install -y ${PACKAGES}

# Fetch rust packages
task -v data-plane:fetch TARGET=${RUSTARCH}
EOF

# Build debug application
RUN <<EOF
case ${TARGETARCH} in
    "amd64")
        RUSTARCH=x86_64
        ;;
    "arm64")
        RUSTARCH=aarch64
        ;;
    *)
        echo "Unsupported platform: ${TARGETPLATFORM}"
        exit 1
        ;;
esac

# Build application
task -v data-plane:build:strip TARGET=${RUSTARCH}-unknown-linux-gnu PROFILE=release
mv target/${RUSTARCH}-unknown-linux-gnu target/${TARGETARCH}-unknown-linux-gnu
EOF

# runtime images  - debug executable, debug symbols and, most importantly, a shell :)
FROM debian:bookworm-slim AS slim-debug

ARG TARGETARCH

# copy the build artifacts from the build stage
COPY --from=rust /app/data-plane/target/${TARGETARCH}-unknown-linux-gnu/release/slim /slim
COPY --from=rust /app/data-plane/target/${TARGETARCH}-unknown-linux-gnu/release/slim.dbg /slim.dbg

# runtime images  - release executable
FROM gcr.io/distroless/cc-debian12:nonroot AS slim-release

ARG TARGETARCH

# copy the artifacts from the build stage
COPY --from=rust /app/data-plane/target/${TARGETARCH}-unknown-linux-gnu/release/slim /slim

FROM debian:bookworm-slim AS testutils
ARG TARGETARCH

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
# copy artifacts from build stage
COPY --from=rust /app/data-plane/target/${TARGETARCH}-unknown-linux-gnu/release/workload-gen /app/workload-gen
COPY --from=rust /app/data-plane/target/${TARGETARCH}-unknown-linux-gnu/release/publisher /app/publisher
COPY --from=rust /app/data-plane/target/${TARGETARCH}-unknown-linux-gnu/release/subscriber /app/subscriber

CMD ["/app/workload-gen"]
