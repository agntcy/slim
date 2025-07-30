# Run this example

# Docker Compose Quick Start

To run all services (slim server, mock app server, and mock app client) using Docker Compose:

```sh
cd data-plane/examples
# Build and start all services
DOCKER_BUILDKIT=1 docker compose up --build
```

This will:
- Build all images (slim-server, mock-app-server, mock-app-client)
- Start the slim server
- Start the mock app server
- Start the mock app client (which will send a message)

To stop and remove all containers:

```sh
docker compose down
```

# Manual (Local) Run

You can also run each service locally using Taskfile:

1. In the first terminal, run the slim service:
   ```sh
   task run:server
   ```
2. In the second terminal, run the mock-app server:
   ```sh
   task run:mock-app:server
   ```
3. In the third terminal, run the mock-app client:
   ```sh
   task run:mock-app:client
   ```
