# SLIM Python Bindings

Bindings to call the SLIM APIs from a python program.

## Installation

```bash
pip install slim-bindings
```

## Include as dependency

### With pyproject.toml

```toml
[project]
name = "slim-example"
version = "0.1.0"
description = "Python program using SLIM"
requires-python = ">=3.9"
dependencies = [
    "slim-bindings>=0.1.0"
]
```

### With poetry project

```toml
[tool.poetry]
name = "slim-example"
version = "0.1.0"
description = "Python program using SLIM"

[tool.poetry.dependencies]
python = ">=3.9,<3.14"
slim-bindings = ">=0.1.0"
```

## Example programs

### Server

```python
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
from signal import SIGINT

import slim_bindings


async def run_server(address: str, enable_opentelemetry: bool):
    # init tracing
    slim_bindings.init_tracing(
        {
            "log_level": "debug"
            "opentelemetry": {
                "enabled": enable_opentelemetry
            }
        }
    )

    global slim
    # create new slim object
    slim = await slim_bindings.Slim.new("cisco", "default", "slim")

    # Run as server
    await slim.run_server({"endpoint": address, "tls": {"insecure": True}})


async def main():
    parser = argparse.ArgumentParser(
        description="Command line client for slim server."
    )
    parser.add_argument(
        "-s", "--slim", type=str, help="SLIM address.", default="127.0.0.1:12345"
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
    client_task = asyncio.create_task(
        run_server(args.slim, args.enable_opentelemetry)
    )

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

### PubSub Client

```python
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
import datetime

import slim_bindings

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
    return f"{color.BOLD}{color.CYAN}{message1.capitalize() :<45}{color.END}{message2}"


def split_id(id):
    # Split the IDs into their respective components
    try:
        local_organization, local_namespace, local_agent = id.split("/")
    except ValueError as e:
        print("Error: IDs must be in the format organization/namespace/agent.")
        raise e

    return local_organization, local_namespace, local_agent


async def run_client(local_id, remote_id, address, enable_opentelemetry: bool):
    # init tracing
    slim_bindings.init_tracing(
        {
            "log_level": "info"
            "opentelemetry": {
                "enabled": enable_opentelemetry
            }
        }
    )

    # Split the local IDs into their respective components
    local_organization, local_namespace, local_agent = split_id(local_id)

    # Split the remote IDs into their respective components
    remote_organization, remote_namespace, broadcast_topic = split_id(remote_id)

    name = f"{local_agent}"

    print(f"Creating participant {name}...")

    participant = await slim_bindings.Slim.new(
        local_organization, local_namespace, local_agent
    )

    # Connect to slim server
    _ = await participant.connect(
        {"endpoint": address, "tls": {"insecure": True}}
    )

    # set route for the chat, so that messages can be sent to the other participants
    await participant.set_route(remote_organization, remote_namespace, broadcast_topic)

    # Subscribe to the producer topic
    await participant.subscribe(remote_organization, remote_namespace, broadcast_topic)

    print(f"{name} -> Creating new pubsub sessions...")
    # create pubsubb session. A pubsub session is a just a bidirectional
    # streaming session, where participants are both sender and receivers
    session_info = await participant.create_session(
        slim_bindings.PySessionConfiguration.Streaming(
            slim_bindings.PySessionDirection.BIDIRECTIONAL,
            topic=slim_bindings.PyAgentType(
                remote_organization, remote_namespace, broadcast_topic
            ),
            max_retries=5,
            timeout=datetime.timedelta(seconds=5),
        )
    )

    # define the background task
    async def background_task():
        msg = f"Hello from {local_agent}"

        async with participant:
            while True:
                try:
                    # receive message from session
                    recv_session, msg_rcv = await participant.receive(
                        session=session_info.id
                    )

                    # Check if the message is calling this specific participant
                    # if not, ignore it
                    if local_agent in msg_rcv.decode():
                        # print the message
                        print(f"{name} -> Received message for me: {msg_rcv.decode()}")

                        # send the message to the next participant
                        await participant.publish(
                            recv_session,
                            msg.encode(),
                            remote_organization,
                            remote_namespace,
                            broadcast_topic,
                        )

                        print(f"{name} -> Sending message to all participants: {msg}")
                    else:
                        print(
                            f"{name} -> Receiving message: {msg_rcv.decode()} - not for me."
                        )
                except asyncio.CancelledError:
                    break
                except Exception as e:
                    print(f"{name} -> Error receiving message: {e}")
                    break

    receive_task = asyncio.create_task(background_task())

    async def background_task_keyboard():
        while True:
            user_input = await asyncio.to_thread(input, "message> ")
            if user_input == "exit":
                break

            # Send the message to the all participants
            await participant.publish(
                session_info,
                f"{user_input}".encode(),
                remote_organization,
                remote_namespace,
                broadcast_topic,
            )

        receive_task.cancel()

    send_task = asyncio.create_task(background_task_keyboard())

    # Wait for both tasks to finish
    await asyncio.gather(receive_task, send_task)


async def main():
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
    parser.add_argument(
        "-s",
        "--slim",
        type=str,
        help="Slim address.",
        default="http://127.0.0.1:46357",
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
    await run_client(
        args.local,
        args.remote,
        args.slim,
        args.enable_opentelemetry,
    )


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("Program terminated by user.")
```

### Streaming producer and consumer

```python
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
import datetime
import os

import slim_bindings

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
    return f"{color.BOLD}{color.CYAN}{message1.capitalize() :<45}{color.END}{message2}"


def split_id(id):
    # Split the IDs into their respective components
    try:
        local_organization, local_namespace, local_agent = id.split("/")
    except ValueError as e:
        print("Error: IDs must be in the format organization/namespace/agent.")
        raise e

    return local_organization, local_namespace, local_agent


async def run_client(
    local_id, remote_id, address, producer, enable_opentelemetry: bool
):
    # init tracing
    slim_bindings.init_tracing(
        {
            "log_level": "info"
            "opentelemetry": {
                "enabled": enable_opentelemetry
            }
        }
    )

    # Split the IDs into their respective components
    local_organization, local_namespace, local_agent = split_id(local_id)

    # create new slim object
    slim = await slim_bindings.Slim.new(
        local_organization, local_namespace, local_agent
    )

    # Connect to the service and subscribe for the local name
    print(format_message("connecting to:", address))
    _ = await slim.connect({"endpoint": address, "tls": {"insecure": True}})

    # Split the IDs into their respective components
    remote_organization, remote_namespace, broadcast_topic = split_id(remote_id)

    # Get the local agent instance from env
    instance = os.getenv("SLIM_INSTANCE_ID", local_agent)

    async with slim:
        if producer:
            # Create a route to the remote ID
            await slim.set_route(
                remote_organization, remote_namespace, broadcast_topic
            )

            # create streaming session with default config
            session_info = await slim.create_session(
                slim_bindings.PySessionConfiguration.Streaming(
                    slim_bindings.PySessionDirection.SENDER,
                    topic=None,
                    max_retries=5,
                    timeout=datetime.timedelta(seconds=5),
                )
            )

            # initialize count
            count = 0

            while True:
                try:
                    # Send a message
                    print(
                        format_message(
                            f"{instance} streaming message to {remote_organization}/{remote_namespace}/{broadcast_topic}: ",
                            f"{count}",
                        )
                    )

                    # Send the message and wait for a reply
                    await slim.publish(
                        session_info,
                        f"{count}".encode(),
                        remote_organization,
                        remote_namespace,
                        broadcast_topic,
                    )

                    count += 1
                except Exception as e:
                    print("received error: ", e)
                finally:
                    await asyncio.sleep(1)
        else:
            # subscribe to streaming session
            await slim.subscribe(
                remote_organization, remote_namespace, broadcast_topic
            )

            # Wait for messages and not reply
            while True:
                session_info, _ = await slim.receive()
                print(
                    format_message(
                        f"{instance.capitalize()} received a new session:",
                        f"{session_info.id}",
                    )
                )

                async def background_task(session_info):
                    while True:
                        # Receive the message from the session
                        session, msg = await slim.receive(session=session_info)
                        print(
                            format_message(
                                f"{local_agent.capitalize()} received from {remote_organization}/{remote_namespace}/{broadcast_topic}: ",
                                f"{msg.decode()}",
                            )
                        )

                asyncio.create_task(background_task(session_info.id))


async def main():
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
    parser.add_argument(
        "-s",
        "--slim",
        type=str,
        help="SLIM address.",
        default="http://127.0.0.1:46357",
    )
    parser.add_argument(
        "-t",
        "--enable-opentelemetry",
        action="store_true",
        default=False,
        help="Enable OpenTelemetry tracing.",
    )
    parser.add_argument(
        "-p",
        "--producer",
        action="store_true",
        default=False,
        help="Enable producer mode.",
    )

    args = parser.parse_args()

    # Run the client with the specified local ID, remote ID, and optional message
    await run_client(
        args.local,
        args.remote,
        args.slim,
        args.producer,
        args.enable_opentelemetry,
    )


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("Program terminated by user.")

```

### Fire And Forget sender and receiver

```python
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
import os

import slim_bindings


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
    return f"{color.BOLD}{color.CYAN}{message1.capitalize() :<45}{color.END}{message2}"


def split_id(id):
    # Split the IDs into their respective components
    try:
        local_organization, local_namespace, local_agent = id.split("/")
    except ValueError as e:
        print("Error: IDs must be in the format organization/namespace/agent.")
        raise e

    return local_organization, local_namespace, local_agent


async def run_client(
    local_id: str,
    remote_id: str,
    message: str,
    address: str,
    iterations: int,
    enable_opentelemetry: bool,
):
    # init tracing
    slim_bindings.init_tracing(
        {
            "log_level": "info"
            "opentelemetry": {
                "enabled": enable_opentelemetry
            }
        }
    )

    local_organization, local_namespace, local_agent = split_id(local_id)

    # create new slim object
    slim = await slim_bindings.Slim.new(
        local_organization, local_namespace, local_agent
    )

    # Connect to remote slim server
    print(format_message(f"connecting to: {address}"))
    _ = await slim.connect({"endpoint": address, "tls": {"insecure": True}})

    # Get the local agent instance from env
    instance = os.getenv("SLIM_INSTANCE_ID", local_agent)

    async with slim:
        if message:
            # Split the IDs into their respective components
            remote_organization, remote_namespace, remote_agent = split_id(remote_id)

            # Create a route to the remote ID
            await slim.set_route(remote_organization, remote_namespace, remote_agent)

            # create a session
            session = await slim.create_session(
                slim_bindings.PySessionConfiguration.FireAndForget()
            )

            for i in range(0, iterations):
                try:
                    # Send the message
                    await slim.publish(
                        session,
                        message.encode(),
                        remote_organization,
                        remote_namespace,
                        remote_agent,
                    )
                    print(format_message(f"{instance} sent:", message))

                    # Wait for a reply
                    session_info, msg = await slim.receive(session=session.id)
                    print(
                        format_message(
                            f"{instance.capitalize()} received (from session {session_info.id}):",
                            f"{msg.decode()}",
                        )
                    )
                except Exception as e:
                    print("received error: ", e)

                await asyncio.sleep(1)
        else:
            # Wait for a message and reply in a loop
            while True:
                session_info, _ = await slim.receive()
                print(
                    format_message(
                        f"{instance.capitalize()} received a new session:",
                        f"{session_info.id}",
                    )
                )

                async def background_task(session_id):
                    while True:
                        # Receive the message from the session
                        session, msg = await slim.receive(session=session_id)
                        print(
                            format_message(
                                f"{instance.capitalize()} received (from session {session_id}):",
                                f"{msg.decode()}",
                            )
                        )

                        ret = f"{msg.decode()} from {instance}"

                        await slim.publish_to(session, ret.encode())
                        print(format_message(f"{instance.capitalize()} replies:", ret))

                asyncio.create_task(background_task(session_info.id))


async def main():
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
        "-s",
        "--slim",
        type=str,
        help="SLIM address.",
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
    await run_client(
        args.local,
        args.remote,
        args.message,
        args.slim,
        args.iterations,
        args.enable_opentelemetry,
    )


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("Program terminated by user.")
```

### Request/Response Requester and Responder

```python
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
import os

import slim_bindings


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
    return f"{color.BOLD}{color.CYAN}{message1.capitalize() :<45}{color.END}{message2}"


def split_id(id):
    # Split the IDs into their respective components
    try:
        local_organization, local_namespace, local_agent = id.split("/")
    except ValueError as e:
        print("Error: IDs must be in the format organization/namespace/agent.")
        raise e

    return local_organization, local_namespace, local_agent


async def run_client(local_id, remote_id, message, address, enable_opentelemetry: bool):
    # init tracing
    slim_bindings.init_tracing(
        {
            "log_level": "info"
            "opentelemetry": {
                "enabled": enable_opentelemetry
            }
        }
    )

    # Split the IDs into their respective components
    local_organization, local_namespace, local_agent = split_id(local_id)

    # create new slim object
    slim = await slim_bindings.Slim.new(
        local_organization, local_namespace, local_agent
    )

    # Connect to the service and subscribe for the local name
    print(format_message("connecting to:", address))
    _ = await slim.connect({"endpoint": address, "tls": {"insecure": True}})

    # Get the local agent instance from env
    instance = os.getenv("SLIM_INSTANCE_ID", local_agent)

    async with slim:
        if message:
            # Split the IDs into their respective components
            remote_organization, remote_namespace, remote_agent = split_id(remote_id)

            # Create a route to the remote ID
            await slim.set_route(remote_organization, remote_namespace, remote_agent)

            # create a session
            session = await slim.create_session(
                slim_bindings.PySessionConfiguration.RequestResponse()
            )

            try:
                # Send the message and wait for a reply
                session_info, reply = await slim.request_reply(
                    session,
                    message.encode(),
                    remote_organization,
                    remote_namespace,
                    remote_agent,
                )
                print(format_message(f"{instance} sent:", message))

                # Print reply and exit
                print(
                    format_message(
                        f"{instance} received (from session {session_info.id}):",
                        f"{reply.decode()}",
                    )
                )

            except Exception as e:
                print("received error: ", e)
        else:
            # Wait for a message and reply in a loop
            while True:
                session_info, _ = await slim.receive()
                print(
                    format_message(
                        f"{instance.capitalize()} received a new session:",
                        f"{session_info.id}",
                    )
                )

                async def background_task(session_id):
                    while True:
                        # Receive the message from the session
                        session, msg = await slim.receive(session=session_id)
                        print(
                            format_message(
                                f"{instance.capitalize()} received (from session {session_id}):",
                                f"{msg.decode()}",
                            )
                        )

                        ret = f"{msg.decode()} from {instance}"

                        await slim.publish_to(session, ret.encode())
                        print(format_message(f"{instance.capitalize()} replies:", ret))

                asyncio.create_task(background_task(session_info.id))


async def main():
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
        "-s",
        "--slim",
        type=str,
        help="SLIM address.",
        default="http://127.0.0.1:46357",
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
    await run_client(
        args.local,
        args.remote,
        args.message,
        args.slim,
        args.enable_opentelemetry,
    )


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("Program terminated by user.")
```
