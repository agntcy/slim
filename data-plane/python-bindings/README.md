# Gateway Python Bindings

Bindings to call the gateway APIs from a python program.

## Installation

```bash
pip install agp-bindings
```

For Windows, see section below.

## Include as dependency

### With pyproject.toml

```toml
[project]
name = "agw-example"
version = "0.1.0"
description = "Python program using AGW"
requires-python = ">=3.9"
dependencies = [
    "agp-bindings>=0.1.0"
]
```

### With poetry project

```toml
[tool.poetry]
name = "agw-example"
version = "0.1.0"
description = "Python program using AGW"

[tool.poetry.dependencies]
python = ">=3.9,<3.14"
agp-bindings = ">=0.1.0"
```

## Example programs

### Server

```python
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
from signal import SIGINT

import agp_bindings
from agp_bindings import GatewayConfig

# Create a service
gateway = agp_bindings.Gateway()


async def run_server(address: str, enable_opentelemetry: bool):
    # init tracing
    agp_bindings.init_tracing(log_level="debug", enable_opentelemetry=enable_opentelemetry)

    # Configure gateway
    config = GatewayConfig(endpoint=address, insecure=True)
    gateway.configure(config)

    # Run as server
    await gateway.serve()


async def main():
    parser = argparse.ArgumentParser(
        description="Command line client for gateway server."
    )
    parser.add_argument(
        "-g", "--gateway", type=str, help="Gateway address.", default="127.0.0.1:12345"
    )
    parser.add_argument(
        "--enable-opentelemetry",
        "-t",
        action="store_true",
        default=False,
        help="Enable OpenTelemetry tracing.",
    )

    args = parser.parse_args()

    # Create an asyncio event to keep the loop running until interrupted
    stop_event = asyncio.Event()

    # Define a shutdown handler to set the event when interrupted
    def shutdown():
        print("\nShutting down...")
        stop_event.set()

    # Register the shutdown handler for SIGINT
    loop = asyncio.get_running_loop()
    loop.add_signal_handler(SIGINT, shutdown)

    # Run the client task
    client_task = asyncio.create_task(run_server(args.gateway, args.enable_opentelemetry))

    # Wait until the stop event is set
    await stop_event.wait()

    # Cancel the client task
    client_task.cancel()
    try:
        await client_task
    except asyncio.CancelledError:
        pass


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("Program terminated by user.")
```

### Client

```python
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
import time

import agp_bindings
from agp_bindings import GatewayConfig


class color:
    PURPLE = "\033[95m"
    CYAN = "\033[96m"
    DARKCYAN = "\033[36m"
    BLUE = "\033[94m"
    GREEN = "\033[92m"
    YELLOW = "\033[93m"
    RED = "\033[91m"
    BOLD = "\033[1m"
    UNDERLINE = "\033[4m"
    END = "\033[0m"


def format_message(message1, message2):
    return f"{color.BOLD}{color.CYAN}{message1.capitalize()}{color.END}\t {message2}"


async def run_client(
    local_id, remote_id, message, address, iterations, enable_opentelemetry: bool
):
    # init tracing
    agp_bindings.init_tracing(
        log_level="debug", enable_opentelemetry=enable_opentelemetry
    )

    # Split the IDs into their respective components
    try:
        local_organization, local_namespace, local_agent = local_id.split("/")
    except ValueError:
        print("Error: IDs must be in the format organization/namespace/agent.")
        return

    # Define the service based on the local agent
    gateway = agp_bindings.Gateway()

    # Configure gateway
    config = GatewayConfig(endpoint=address, insecure=True)
    gateway.configure(config)

    # Connect to the gateway server
    local_agent_id = await gateway.create_agent(
        local_organization, local_namespace, local_agent
    )

    # Connect to the service and subscribe for the local name
    _ = await gateway.connect()
    await gateway.subscribe(
        local_organization, local_namespace, local_agent, local_agent_id
    )

    if message:
        if not iterations:
            iterations = 1

        # Split the IDs into their respective components
        try:
            remote_organization, remote_namespace, remote_agent = remote_id.split("/")
        except ValueError:
            print("Error: IDs must be in the format organization/namespace/agent.")
            return

        # Create a route to the remote ID
        await gateway.set_route(remote_organization, remote_namespace, remote_agent)

        # create a session
        session_id = await gateway.create_session(
            agp_bindings.PySessionType.FireAndForget
        )

        for i in range(0, iterations):
            try:
                # Send the message
                await gateway.publish(
                    session_id,
                    message.encode(),
                    remote_organization,
                    remote_namespace,
                    remote_agent,
                )
                print(format_message(f"{local_agent.capitalize()} sent:", message))

                # Wait for a reply
                src, msg = await gateway.receive()
                print(
                    format_message(
                        f"{local_agent.capitalize()} received:", msg.decode()
                    )
                )
            except Exception as e:
                print("received error: ", e)

            time.sleep(5)
    else:
        # Wait for a message and reply in a loop
        while True:
            session_info, src, msg = await gateway.receive()
            print(
                format_message(
                    f"{local_agent.capitalize()} received: {msg.decode()} from session {session_info.id}"
                )
            )

            ret = f"Echo from {local_agent}: {msg.decode()}"

            await gateway.publish_to(ret.encode(), src)
            print(format_message(f"{local_agent.capitalize()} replies:", ret))


def main():
    parser = argparse.ArgumentParser(
        description="Command line client for message passing."
    )
    parser.add_argument(
        "-l",
        "--local",
        type=str,
        help="Local ID in the format organization/namespace/agent.",
    )
    parser.add_argument(
        "-r",
        "--remote",
        type=str,
        help="Remote ID in the format organization/namespace/agent.",
    )
    parser.add_argument("-m", "--message", type=str, help="Message to send.")
    parser.add_argument(
        "-g",
        "--gateway",
        type=str,
        help="Gateway address.",
        default="http://127.0.0.1:46357",
    )
    parser.add_argument(
        "-i",
        "--iterations",
        type=int,
        help="Number of messages to send, one per second.",
    )
    parser.add_argument(
        "-t",
        "--enable-opentelemetry",
        action="store_true",
        default=False,
        help="Enable OpenTelemetry tracing.",
    )

    args = parser.parse_args()

    # Run the client with the specified local ID, remote ID, and optional message
    asyncio.run(
        run_client(
            args.local,
            args.remote,
            args.message,
            args.gateway,
            args.iterations,
            args.enable_opentelemetry,
        )
    )


if __name__ == "__main__":
    main()
```
