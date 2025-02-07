# Data plane

Rust-based forwarder for agent-to-agent communications.

## Build

### Local binary

Set the build profile you prefer. Available options:

- debug
- release

```bash
PROFILE=release
# PROFILE=debug

task data-plane:build PROFILE=${PROFILE}
```

This will build a local binary of the gateway.

### Container image

To run a multiarch image of the gateway (linux/arm64 & linux/amd64):

```
REPO_ROOT="$(git rev-parse --show-toplevel)"
docker build -t gateway -f "${REPO_ROOT}/data-plane/Dockerfile" --platform linux/amd64,linux/arm64 "${REPO_ROOT}"
```

Or alternatively, with docker buildx bake:

```
pushd $(git rev-parse --show-toplevel) && IMAGE_REPO=gateway IMAGE_TAG=latest docker buildx bake gateway && popd
```

## Run gateway

The gateway can be run in 3 main ways:

- directly as binary (preferred way when deployed as workload in k8s)
- via the rust APIs (check the [sdk-mock example](./examples/src/sdk-mock))
- via the [python bindings](./python-bindings)

The gateway can run in server mode, in client mode or both (i.e. spawning a
server and connecting to another gateway at the same time).

### Server

To run the gateway binary as server, a configuration file is needed to setup the
basic runtime options. Some basic examples are provided in the
[config](./config/) folder:

- [reference](./config/reference/config.yaml) is a reference configuration, with
  comments explaining all the available options.
- [base](./config/base/server-config.yaml) is a base configuration for a server
  without encryption and authentication.
- [tls](./config/tls/server-config.yaml) is a configuration for a server with
  encryption enabled, with no authentication.
- [basic-auth](./config/basic-auth/server-config.yaml) is a configuration for a
  server with encryption and basic auth enabled.
- [mtls](./config/mtls/server-config.yaml) is a configuration for a server
  expecting clients to authenticate with a trusted certificate.

To run the gateway as server:

```bash
MODE=base
# MODE=tls
# MODE=basic-auth; export PASSWORD=12345
# MODE=mtls

cargo run --bin gateway -- --config ./config/${MODE}/server-config.yaml
```

Or, using the container image (assuming the image name is
`gateway/agp`):

```bash
docker run -it \
    -e PASSWORD=${PASSWORD} \
    -v ./config/base/server-config.yaml:/config.yaml \
    gateway/agp /gateway --config /config.yaml
```

### Client

To run the gateway binary as client, you will need to configure it to start one
(or more) clients at startup. and you will need to provide the address of a
remote gateway server. As usually, some configuration examples are available in
the [config](./config/) folder:

- [reference](./config/reference/config.yaml) is a reference configuration, with
  comments explaining all the available options.
- [base](./config/base/client-config.yaml) is a base configuration for a client
  without encryption and authentication.
- [tls](./config/tls/client-config.yaml) is a configuration for a client with
  encryption enabled, with no authentication.
- [basic-auth](./config/basic-auth/client-config.yaml) is a configuration for a
  client with encryption and basic auth enabled.
- [mtls](./config/mtls/client-config.yaml) is a configuration for a client
  connecting to a server with a trusted certificate.

To run the gateway as client:

```bash
MODE=base
# MODE=tls
# MODE=basic-auth; export PASSWORD=12345
# MODE=mtls

cargo run --bin gateway -- --config ./config/${MODE}/client-config.yaml
```

Or, using the container image (assuming the image name is
`gateway/agp`):

```bash
docker run -it \
    -e PASSWORD=${PASSWORD} \
    -v ./config/base/client-config.yaml:/config.yaml \
    gateway/agp /gateway --config /config.yaml
```
