# Dockerfile for running a2acli in K8s jobs
FROM golang:1.24-alpine AS builder

# Install dependencies for CGO
RUN apk add --no-cache gcc musl-dev git task

WORKDIR /build

# Copy the a2acli source
COPY . .

# Build a2acli
RUN task build

# Runtime image
FROM alpine:latest

# Install CA certificates for HTTPS
RUN apk add --no-cache ca-certificates

WORKDIR /app

# Copy the built binary
COPY --from=builder /build/bin/a2acli /app/a2acli

# Create directory for agent cards
RUN mkdir -p /app/.a2aagents

# Make binary executable
RUN chmod +x /app/a2acli

ENTRYPOINT ["/app/a2acli"]
