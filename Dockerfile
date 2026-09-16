# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

# Build container
FROM --platform=${BUILDPLATFORM} rust:1.95-slim-bookworm@sha256:d7482085ff5b415f84dba5647ae71606650bdef00db7aeb69f4b3d170c3e4082 AS rust

SHELL ["/bin/bash", "-euo", "pipefail", "-c"]

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
        cmake \
        ninja-build \
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
WORKDIR /app

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
  --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
  --mount=type=cache,target=/app/target,sharing=locked \
  <<EOF
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
task -v fetch TARGET=${RUSTARCH}

# Build application
task -v build:strip TARGET=${RUSTARCH} PROFILE=release ARGS="--locked --bin slim --bin slim-control-plane --bin channel-manager"
mv target/${RUSTARCH}/release/slim /slim
mv target/${RUSTARCH}/release/slim.dbg /slim.dbg

# Strip and export control plane binary
task -v strip TARGET_BIN=target/${RUSTARCH}/release/slim-control-plane
mv target/${RUSTARCH}/release/slim-control-plane /slim-control-plane
mv target/${RUSTARCH}/release/slim-control-plane.dbg /slim-control-plane.dbg

# Strip and export channel manager binary
task -v strip TARGET_BIN=target/${RUSTARCH}/release/channel-manager
mv target/${RUSTARCH}/release/channel-manager /channel-manager
mv target/${RUSTARCH}/release/channel-manager.dbg /channel-manager.dbg
EOF

# Grab libgcc from the CC image
FROM gcr.io/distroless/cc-debian12@sha256:e5d81ddde149641e2a9ba55be4545bc125c67de07508b03ba4c22e6eb0ded5aa AS libgcc-provider

# Runtime images  - debug executable, debug symbols and, most importantly, a shell :)
FROM debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171 AS slim-debug

ARG TARGETARCH

# copy the build artifacts from the build stage
COPY --from=rust /slim /slim
COPY --from=rust /slim.dbg /slim.dbg

# Runtime images - release executable
FROM gcr.io/distroless/base-nossl-debian12:nonroot@sha256:be40c00dfabd86576d92666e87e406714d5618342de1a0c213ad232de255172e AS slim-release

ARG TARGETARCH

# Copy libgcc from the libgcc-provider image
COPY --from=libgcc-provider /lib/*-linux-gnu/libgcc_s.so.1 /lib/

# copy the artifacts from the build stage
COPY --from=rust /slim /slim

# Runtime image - control plane debug executable, debug symbols and a shell
FROM debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171 AS control-plane-debug

ARG TARGETARCH

# copy the build artifacts from the build stage
COPY --from=rust /slim-control-plane /slim-control-plane
COPY --from=rust /slim-control-plane.dbg /slim-control-plane.dbg

# Runtime image - control plane release executable
FROM gcr.io/distroless/base-nossl-debian12:nonroot@sha256:be40c00dfabd86576d92666e87e406714d5618342de1a0c213ad232de255172e AS control-plane-release

ARG TARGETARCH

# Copy libgcc from the libgcc-provider image
COPY --from=libgcc-provider /lib/*-linux-gnu/libgcc_s.so.1 /lib/

# copy the artifacts from the build stage
COPY --from=rust /slim-control-plane /slim-control-plane

ENTRYPOINT ["/slim-control-plane"]


# Runtime image - channel manager debug executable, debug symbols and a shell
FROM debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171 AS channel-manager-debug

ARG TARGETARCH

# copy the build artifacts from the build stage
COPY --from=rust /channel-manager /channel-manager
COPY --from=rust /channel-manager.dbg /channel-manager.dbg

# Runtime image - channel manager release executable
FROM gcr.io/distroless/base-nossl-debian12:nonroot@sha256:be40c00dfabd86576d92666e87e406714d5618342de1a0c213ad232de255172e AS channel-manager-release

ARG TARGETARCH

# Copy libgcc from the libgcc-provider image
COPY --from=libgcc-provider /lib/*-linux-gnu/libgcc_s.so.1 /lib/

# copy the artifacts from the build stage
COPY --from=rust /channel-manager /channel-manager

ENTRYPOINT ["/channel-manager"]
