FROM golang:1.23 AS builder

WORKDIR /

# https://pkg.go.dev/cmd/cgo
# Some C stuff dosen't work on distroless images
ENV CGO_ENABLED=0
SHELL ["/usr/bin/bash", "-c"]

# Copy all the necessary sources from the repo root
COPY . /ctx
WORKDIR /ctx/control-plane

# Install task runner
RUN curl --location https://taskfile.dev/install.sh -o install.sh && \
    sh install.sh -d

RUN ./bin/task control-plane:control-plane:build

#
## https://github.com/GoogleContainerTools/distroless
FROM gcr.io/distroless/static:nonroot AS control-plane
WORKDIR /

COPY --from=builder /ctx/.dist/bin/control-plane .


ENTRYPOINT ["control-plane"]
