FROM --platform=${BUILDPLATFORM} golang:1.24 AS builder

ARG TARGETOS
ARG TARGETARCH

SHELL ["/usr/bin/bash", "-c"]

WORKDIR /

# Install unzip
RUN apt update && apt install unzip

# Copy all the necessary sources from the repo root
COPY . /ctx
WORKDIR /ctx/control-plane

# Install task runner
RUN sh -c "$(curl --location https://taskfile.dev/install.sh)" -- -d -b /usr/local/bin

RUN CGO_ENABLED=0 GOOS=${TARGETOS} GOARCH=${TARGETARCH} task control-plane:build

## https://github.com/GoogleContainerTools/distroless
FROM gcr.io/distroless/static:nonroot AS control-plane

WORKDIR /

COPY --from=builder /ctx/.dist/bin/control-plane .
COPY --from=builder /ctx/.dist/bin/slimctl .

ENTRYPOINT ["/control-plane"]
