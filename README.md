#
[![Data-plane CI](https://github.com/agntcy/slim/actions/workflows/data-plane.yaml/badge.svg)](https://github.com/agntcy/slim/actions/workflows/data-plane.yaml)
[![Control-plane
CI](https://github.com/agntcy/slim/actions/workflows/control-plane.yaml/badge.svg)](https://github.com/agntcy/slim/actions/workflows/control-plane.yaml)
[![codecov](https://codecov.io/gh/agntcy/slim/branch/main/graph/badge.svg)](https://codecov.io/gh/agntcy/slim)
[![Coverage](https://img.shields.io/badge/Coverage-passing-brightgreen)](https://codecov.io/gh/agntcy/slim)

# SLIM

SLIM (Secure Low-Latency Interactive Messaging) facilitates communication
between AI agents. It supports various communication patterns such as unicast,
anycast and multicast groups. Built on the gRPC framework, SLIM ensures secure
secure and scalable interactions among agents.


## Features

- **Security**: Employs authentication, authorization, and end-to-end encryption
  with [MLS](https://datatracker.ietf.org/doc/rfc9420/) to protect data privacy
  and integrity.
- **Scalability**: Designed to handle a large number of concurrent connections
  and messages, making it suitable for high-load scenarios.
- **Flexibility**: Supports various communication patterns, including unicast,
  anycast, and multicast, to accommodate different use cases.
- **Performance**: Optimized for low-latency communication, ensuring quick
  message delivery and responsiveness.

## Source tree

Main software components:

- [data-plane](./data-plane): client and cloud components for efficient message
  forwarding among agents
- [control-plane](./control-plane): cloud services to manage control-plane ops
  carried out by agents

## Prerequisites

To build the project and work with the code, you will need the following
installed in your system:

### [Taskfile](https://taskfile.dev/)

Taskfile is required to run all the build operations. Follow the
[installation](https://taskfile.dev/installation/) instructions in the Taskfile
documentations to find the best installation method for your system.

<details>
  <summary>with brew</summary>

  ```bash
  brew install go-task
  ```
</details>
<details>
  <summary>with curl</summary>

  ```bash
  sh -c "$(curl --location https://taskfile.dev/install.sh)" -- -d -b ~/.local/bin
  ```
</details>


### [Rust](https://rustup.rs/)

The data-plane components are implemented in rust. Install with rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### [Golang](https://go.dev/doc/install)

The control-plane components are implemented in golang. Follow the installation
instructions in the golang website.

## Artifacts distribution

### [Crates](./data-plane)

See https://crates.io/users/artifacts-agntcy

```bash
cargo install slim
```

### [Container images](./data-plane/Dockerfile)

```bash
docker pull ghcr.io/agntcy/slim:latest
```

### [Helm charts](./deploy/charts/slim)

```bash
helm pull ghcr.io/agntcy/slim/helm/slim:latest
```

### [Pypi packages](./data-plane/python/bindings)

```bash
pip install slim-bindings
```

### Copyright Notice

[Copyright Notice and License](./LICENSE.md)

Distributed under Apache 2.0 License. See LICENSE for more information.

Copyright AGNTCY Contributors (https://github.com/agntcy)
