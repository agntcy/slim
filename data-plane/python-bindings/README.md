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

## Windows

In the case of Windows, `agp_bindings` will need to be compiled locally.

## 1. Build Requirements

1. **Rust Toolchain**  
   - [Install Rust](https://www.rust-lang.org/tools/install)  
   - Make sure `cargo`, `rustc` are in your PATH.
2. **Python**  
   - Install a Python version (3.9–3.13).
   - **Optional**: Use [pyenv-win](https://github.com/pyenv-win/pyenv-win) or another Python version manager if you need multiple versions.
3. **Maturin**  
   - You can install Maturin via `pip install maturin` or `cargo install maturin`.
4. **Task/`taskfile`**  
   - If you’re using [go-task](https://taskfile.dev/) or a similar tool, make sure it’s installed.  
   - Alternatively, if `task` is just a script/alias in your project, ensure it’s executable.
5. Install/verify you have "Desktop development with C++" in Visual Studio.

   In particular, make sure you have the following key items checked, which are most important for MSVC + CMake builds:

      - MSVC v143 – VS 2022 C++ x64/x86 build tools
      - C++ CMake Tools for Windows
      - Windows 11 SDKs

## 2. Run the Build Locally

Clone <https://github.com/agntcy/agp> and change to folder `data-plane\python-bindings`

Inside this folder (where the Taskfile is), you can run:

```bash
# List all available tasks
task

# Build Python bindings in debug mode:
task python-bindings:build
```

This will build the wheel under a temporary folder that will be removed immediately but serves to test if the toolchain is correctly setup.

You should see a similar output:

```Powershell
Built wheel for CPython 3.13 to C:\Users\dummy\AppData\Local\Temp\.tmpYMjkNn\agp_bindings-0.1.7-cp313-cp313-win_amd64.whl
```

### Build Bindings to Dist Directory

1. Disable any cloud data syncing programs, such as `OneDrive` or `Dropbox`, that may be monitoring the folder you are working on. Otherwise, the build will fail due to file locking issues.

2. Execute maturin

    ```powershell
    maturin build --release --out dist
    ```

    You should see a similar output:

    ```Powershell
    📦 Built wheel for CPython 3.13 to dist\agp_bindings-0.1.7-cp313-cp313-win_amd64.whl
    ```

## 3. Install Wheel and Verify the Installation

```Powershell
pip install .\dist\agp_bindings-0.1.7-cp313-cp313-win_amd64.whl
```

### Verify

It is very important that the path displayed **points to your virtual environment** and not to the folder `agp_bindings`

```Powershell
cd agp\data-plane\
python -c "import agp_bindings; print(agp_bindings.__file__)"
```

That should show a path to the installed agp_bindings in your virtual environment’s Lib\site-packages. Example:

```Powershell
agp\data-plane\python-bindings\.venv\Lib\site-packages\agp_bindings\__init__.py
```

## 4. Troubleshooting on Windows

- **MSVC / cl.exe** not found:  
  Make sure you installed the **"Desktop development with C++"** workload in Visual Studio Installer and that you’re building in a Developer Command Prompt.
- **File Tracker (FTK1011) or Temp Directory** errors:  
  If you see warnings about building from `Temp`, try changing or shortening your Windows temp directory [as discussed in previous steps](https://docs.microsoft.com/en-us/cpp/build/reference/filetracker).
- **There’s an Old _agp_bindings.pyd or a Naming Conflict**

    Sometimes you can end up with two .pyd files or an out-of-date file in `agp_bindings`. This can confuse Python or Maturin. If you see multiple _agp_bindings.cpXYZ-win_amd64.pyd files, remove the duplicates.

---

## Example programs

### Server

```python
# SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
from signal import SIGINT

import agp_bindings

# Create a service
gateway = agp_bindings.Gateway()


async def run_server(address: str):
    # init tracing with debug
    agp_bindings.init_tracing(log_level="debug")

    # Run as server
    await gateway.serve(address, insecure=True)


async def main():
    parser = argparse.ArgumentParser(
        description="Command line client for gateway server."
    )
    parser.add_argument(
        "-g", "--gateway", type=str, help="Gateway address.", default="127.0.0.1:12345"
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
    client_task = asyncio.create_task(run_server(args.gateway))

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
# SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
import time

import agp_bindings


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


async def run_client(local_id, remote_id, message, address):
    # init tracing
    agp_bindings.init_tracing()

    # Split the IDs into their respective components
    try:
        local_organization, local_namespace, local_agent = local_id.split("/")
    except ValueError:
        print("Error: IDs must be in the format organization/namespace/agent.")
        return

    # Define the service based on the local agent
    gateway = agp_bindings.Gateway()

    # Connect to the gateway server
    local_agent_id = await gateway.create_agent(
        local_organization, local_namespace, local_agent
    )

    # Connect to the service and subscribe for the local name
    _ = await gateway.connect(address, insecure=True)
    await gateway.subscribe(
        local_organization, local_namespace, local_agent, local_agent_id
    )

    if message:
        # Split the IDs into their respective components
        try:
            remote_organization, remote_namespace, remote_agent = remote_id.split("/")
        except ValueError:
            print("Error: IDs must be in the format organization/namespace/agent.")
            return

        # Create a route to the remote ID
        await gateway.set_route(remote_organization, remote_namespace, remote_agent)

        # Send the message
        await gateway.publish(
            message.encode(), remote_organization, remote_namespace, remote_agent
        )
        print(format_message(f"{local_agent.capitalize()} sent:", message))

        # Wait for a reply
        src, msg = await gateway.receive()
        print(format_message(f"{local_agent.capitalize()} received:", msg.decode()))
    else:
        # Wait for a message and reply in a loop
        while True:
            src, msg = await gateway.receive()
            print(format_message(f"{local_agent.capitalize()} received:", msg.decode()))

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
        default="http://127.0.0.1:12345",
    )

    args = parser.parse_args()

    # Run the client with the specified local ID, remote ID, and optional message
    asyncio.run(run_client(args.local, args.remote, args.message, args.gateway))


if __name__ == "__main__":
    main()
```
