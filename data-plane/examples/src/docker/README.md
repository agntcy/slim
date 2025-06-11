# Docker Setup for Slim Examples

This directory contains Docker configurations for running the Slim server, mock agent server, and mock agent client.

## Dockerfiles

Three separate Dockerfiles are provided:

1. `Dockerfile.slim` - Builds and runs the Slim server
2. `Dockerfile.mock-agent-server` - Builds and runs the mock agent in server mode
3. `Dockerfile.mock-agent-client` - Builds and runs the mock agent in client mode (with a message)

## Using Docker Compose

The easiest way to run all three components together is to use Docker Compose:

```bash
cd data-plane/examples/src/docker
docker-compose up --build
```

This will:
1. Build all three images
2. Start the Slim server
3. Start the mock agent server
4. Start the mock agent client that will send a message

## Building Images Individually

You can also build and run each image separately:

### Slim Server

```bash
# Build
docker build -t slim-server -f data-plane/examples/src/docker/Dockerfile.slim .

# Run
docker run -p 50051:50051 -v $(pwd)/data-plane/config/base/server-config.yaml:/config/server-config.yaml slim-server
```

### Mock Agent Server

```bash
# Build
docker build -t mock-agent-server -f data-plane/examples/src/docker/Dockerfile.mock-agent-server .

# Run
docker run -v $(pwd)/data-plane/config/base/client-config.yaml:/config/client-config.yaml mock-agent-server
```

### Mock Agent Client

```bash
# Build
docker build -t mock-agent-client -f data-plane/examples/src/docker/Dockerfile.mock-agent-client .

# Run
docker run -v $(pwd)/data-plane/config/base/client-config.yaml:/config/client-config.yaml -e DEFAULT_MESSAGE="hello world" mock-agent-client
```

## Configuration

All containers mount configuration files from the host system. You can modify these files in the `data-plane/config/base/` directory:

- `server-config.yaml` - Configuration for the Slim server
- `client-config.yaml` - Configuration for the mock agents

## Network Configuration

The Docker Compose setup creates a shared network for all three services, allowing them to communicate with each other. When running containers individually, you might need to create and configure a Docker network manually.
