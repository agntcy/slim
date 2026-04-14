# Dockerfile for running a2acli in K8s jobs
FROM golang:1.25-bookworm AS builder

# Install dependencies for CGO
RUN apt-get update && apt-get install -y gcc git && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy go module files first for better caching
COPY go.mod go.sum ./
RUN go mod download

# Copy the rest of the source
COPY . .

# Download slim-bindings-go native libraries (setup step)
RUN go run github.com/agntcy/slim-bindings-go/cmd/slim-bindings-setup

# Build a2acli
RUN CGO_ENABLED=1 go build \
    -ldflags "-X google.golang.org/protobuf/reflect/protoregistry.conflictPolicy=ignore" \
    -o bin/a2acli .

ENTRYPOINT ["bin/a2acli"]
