# Agent Gateway Protocol

The Agent Gateway Protocol (AGP) facilitates communication between AI agents.
It supports various communication patterns such as request-response,
publish-subscribe, fire-and-forget, and streaming. Built on the gRPC framework,
the Agent Gateway ensures secure and scalable interactions among agents.


## Features

- **Request-Response**: Supports synchronous communication between agents.
- **Publish-Subscribe**: Allows agents to publish messages to topics and subscribe to receive messages from topics.
- **Fire-and-Forget**: Enables agents to send messages without waiting for a response.
- **Streaming**: Supports both unidirectional and bidirectional streaming.
- **Security**: Employs authentication, authorization, and end-to-end encryption to protect data privacy and integrity.

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

## Copyright Notice

[Copyright Notice and License](./LICENSE.md)
Copyright (c) 2025 Cisco and/or its affiliates.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.