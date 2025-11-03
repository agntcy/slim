# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

# Build container
FROM --platform=${BUILDPLATFORM} rust:1.90-slim-bookworm AS rust

SHELL ["/bin/bash", "-c"]

ARG TARGETARCH

RUN <<EOF
case ${TARGETARCH} in
    "amd64")
        PACKAGES="gcc-x86-64-linux-gnu g++-x86-64-linux-gnu"
        ;;
    "arm64")
        PACKAGES="gcc-aarch64-linux-gnu g++-aarch64-linux-gnu"
        ;;
    *)
        echo "Unsupported platform: ${TARGETPLATFORM}"
        exit 1
        ;;
esac

DEBIAN_FRONTEND=noninteractive \
    apt-get update && \
    apt-get install --no-install-recommends -y \
        curl \
        file \
        make \
        unzip \
        git \
        lsb-release \
        software-properties-common \
        gnupg \
        pkg-config \
        ${PACKAGES}

curl -L -o /tmp/llvm.sh https://apt.llvm.org/llvm.sh
chmod +x /tmp/llvm.sh
/tmp/llvm.sh 19

curl -1sLf 'https://dl.cloudsmith.io/public/task/task/setup.deb.sh' | bash
apt-get install -y task
EOF

# Copy source code
COPY . /app
WORKDIR /app/data-plane

RUN <<EOF
case ${TARGETARCH} in
    "amd64")
        RUSTARCH=x86_64-unknown-linux-gnu
        ;;
    "arm64")
        RUSTARCH=aarch64-unknown-linux-gnu
        ;;
    *)
        echo "Unsupported platform: ${TARGETPLATFORM}"
        exit 1
        ;;
esac

# Fetch rust packages
task -v data-plane:fetch TARGET=${RUSTARCH}

# Build application
task -v data-plane:build:strip TARGET=${RUSTARCH} PROFILE=release
mv target/${RUSTARCH}/release/slim /slim
mv target/${RUSTARCH}/release/slim.dbg /slim.dbg
mv target/${RUSTARCH}/release/workload-gen /workload-gen
mv target/${RUSTARCH}/release/publisher /publisher
mv target/${RUSTARCH}/release/subscriber /subscriber
EOF

# runtime images  - debug executable, debug symbols and, most importantly, a shell :)
FROM debian:bookworm-slim AS slim-debug

ARG TARGETARCH

RUN ls -l /app/data-plane/target/

# copy the build artifacts from the build stage
COPY --from=rust /slim /slim
COPY --from=rust /slim.dbg /slim.dbg

# runtime images  - release executable
FROM gcr.io/distroless/cc-debian12:nonroot AS slim-release

ARG TARGETARCH

# copy the artifacts from the build stage
COPY --from=rust /slim /slim

FROM debian:bookworm-slim AS testutils
ARG TARGETARCH

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
# copy artifacts from build stage
COPY --from=rust /workload-gen /app/workload-gen
COPY --from=rust /publisher /app/publisher
COPY --from=rust /subscriber /app/subscriber

CMD ["/app/workload-gen"]
