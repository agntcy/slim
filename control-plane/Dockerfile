FROM --platform=${BUILDPLATFORM} golang:1.25.6 AS builder

ARG TARGETOS
ARG TARGETARCH

SHELL ["/usr/bin/bash", "-c"]

WORKDIR /

# Install unzip and xz-utils (for tar.xz extraction)
RUN apt update && apt install -y unzip curl xz-utils

# Copy all the necessary sources from the repo root
COPY . /ctx
WORKDIR /ctx/control-plane

# Install task runner
RUN sh -c "$(curl --location https://taskfile.dev/install.sh)" -- -d -b /usr/local/bin

# # Build control-plane binaries (control-plane, token-service, slimctl)
# # control-plane and token-service don't need CGO
RUN GOOS=${TARGETOS} GOARCH=${TARGETARCH} CGO_ENABLED=0 task control-plane:control-plane:build
RUN GOOS=${TARGETOS} GOARCH=${TARGETARCH} CGO_ENABLED=0 task control-plane:token-service:build

## https://github.com/GoogleContainerTools/distroless
FROM --platform=${TARGETOS}/${TARGETARCH} gcr.io/distroless/static:nonroot AS control-plane

WORKDIR /

COPY --from=builder /ctx/.dist/bin/control-plane .

ENTRYPOINT ["/control-plane"]
