# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

FROM --platform=${BUILDPLATFORM} golang:1.23-bullseye AS development-env

SHELL [ "/bin/bash", "-c" ]

ARG USERNAME=ubuntu
ARG USER_UID=1000
ARG USER_GID=${USER_UID}
ARG TARGETARCH
ENV PATH=/usr/local/go/bin:/home/${USERNAME}/go/bin:${PATH}

RUN apt-get update && \
  apt-get -y install git sudo unzip curl bc && \
  sh -c "$(curl --location https://taskfile.dev/install.sh)" -- -d -b /usr/local/bin

# Add non-root user
RUN groupadd --gid ${USER_GID} ${USERNAME} && \
  useradd -s /bin/bash --uid ${USER_UID} --gid ${USER_GID} -m ${USERNAME} && \
  mkdir -p /etc/sudoers.d && \
  echo ${USERNAME} ALL=\(root\) NOPASSWD:ALL >/etc/sudoers.d/${USERNAME} && \
  chmod 0440 /etc/sudoers.d/${USERNAME}

RUN <<EOF
# Download grpc health check
curl -o /bin/grpc_health_probe -sSL https://github.com/grpc-ecosystem/grpc-health-probe/releases/download/v0.4.28/grpc_health_probe-linux-${TARGETARCH}
chmod +x /bin/grpc_health_probe
EOF

USER ${USERNAME}
WORKDIR /home/${USERNAME}

########################
# Build Token Service ##
########################
FROM --platform=${BUILDPLATFORM} development-env AS build-slim-tks

ARG USERNAME=ubuntu
ARG TARGETOS
ARG TARGETARCH

COPY --chown=${USERNAME}:${USERNAME} . ./slim
WORKDIR /home/${USERNAME}/slim

RUN CGO_ENABLED=0 GOOS=${TARGETOS} GOARCH=${TARGETARCH} task control-plane:token-service:build

########################
# Production Image    ##
########################
FROM gcr.io/distroless/static:nonroot AS slim-tks

ARG USERNAME=ubuntu

COPY --from=build-slim-tks /home/${USERNAME}/slim/.dist/bin/slim-tks /slim-tks
COPY --from=build-slim-tks /bin/grpc_health_probe /grpc_health_probe
