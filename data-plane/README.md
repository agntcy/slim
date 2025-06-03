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

This will build a local binary of SLIM.

### Container image

To run a multiarch image of SLIM (linux/arm64 & linux/amd64):

```bash
REPO_ROOT="$(git rev-parse --show-toplevel)"
docker build -t slim -f "${REPO_ROOT}/data-plane/Dockerfile" --platform linux/amd64,linux/arm64 "${REPO_ROOT}"
```

Or alternatively, with docker buildx bake:

```bash
pushd $(git rev-parse --show-toplevel) && IMAGE_REPO=slim IMAGE_TAG=latest docker buildx bake slim && popd
```

---

### Container Image on Windows

The container image build process was tested on:

- Windows 10 + Hyper-V
- windows 11 + WSL 2

The instructions below assume [Powershell 7](<https://learn.microsoft.com/en-us/powershell/scripting/install/installing-powershell-on-windows?view=powershell-7.5>) environment.

#### Set environment variable

```Powershell
$env:REPO_ROOT = "<path-to-repo>"
```

Example:

```Powershell
$env:REPO_ROOT = "C:\Users\<dummy>\slim"
```

#### Convert Line endings

To convert line endings in **VSCode** from **CRLF** (Windows-style) to **LF** (Unix-style), follow these steps:

1. Open the **Dockerfile** (or any file you want to convert) in VSCode.
2. Look at the **bottom-right corner** of the VSCode window.
3. You should see something like `CRLF` (Carriage Return + Line Feed).
4. Click on `CRLF`, and a small menu will appear.
5. Select **LF (Line Feed)** from the menu.
6. Save the file (`Ctrl + S` or `Cmd + S` on Mac).

#### Build Container

Notice that there is no arm64 on the command below, otherwise it will fail even if using `buildx`.

```Powershell
docker buildx build `
    -t slim `
    -f "$env:REPO_ROOT\data-plane\Dockerfile" `
    --platform linux/amd64 `
    "$env:REPO_ROOT"
```

If everything goes well you should see an output similar to:

```bash
...
 => => naming to docker.io/library/slim:latest                                                             0.0s
 => => unpacking to docker.io/library/slim:latest
 ```

## Run SLIM

SLIM can be run in 3 main ways:

- directly as binary (preferred way when deployed as workload in k8s)
- via the rust APIs (check the [sdk-mock example](./examples/src/sdk-mock))
- via the [python bindings](./python-bindings)

SLIM can run in server mode, in client mode or both (i.e. spawning a
server and connecting to another SLIM instance at the same time).

### Server

To run SLIM binary as server, a configuration file is needed to setup the
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

To run SLIM as server:

```bash
MODE=base
# MODE=tls
# MODE=basic-auth; export PASSWORD=12345
# MODE=mtls

cargo run --bin slim -- --config ./config/${MODE}/server-config.yaml
```

Or, using the container image (assuming the image name is
`slim`):

```bash
docker run -it \
    -e PASSWORD=${PASSWORD} \
    -v ./config/base/server-config.yaml:/config.yaml \
    slim /slim --config /config.yaml
```

---

### Docker on Windows

The instructions below assume [Powershell 7](<https://learn.microsoft.com/en-us/powershell/scripting/install/installing-powershell-on-windows?view=powershell-7.5>) environment.

#### **1. Set the Password as an Environment Variable**

```powershell
$env:PASSWORD = "your_secure_password"
```

This sets the environment variable `PASSWORD` for the current session.

#### **2. Run the `docker run` Command**

Since PowerShell doesn’t support `${PASSWORD}`, replace it with `$env:PASSWORD`:

:bomb: **Notice we are exposing port `46357` to the host and it matches the one inside `${PWD}/config/base/server-config.yaml`**

```powershell
docker run -it `
    -e PASSWORD=$env:PASSWORD `
    -v ${PWD}/config/base/server-config.yaml:/config.yaml `
    -p 46357:46357 `
    slim:latest /slim --config /config.yaml
```

#### **Persistent Configuration (Optional)**

If you want the password to persist across sessions:

```powershell
[System.Environment]::SetEnvironmentVariable("PASSWORD", "your_secure_password", "User")
```

This makes `PASSWORD` available across all PowerShell sessions.

---

### Client

To run the SLIM binary as client, you will need to configure it to start one
(or more) clients at startup, and you will need to provide the address of a
remote SLIM server. As usually, some configuration examples are available in
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

To run SLIM as client:

```bash
MODE=base
# MODE=tls
# MODE=basic-auth; export PASSWORD=12345
# MODE=mtls

cargo run --bin slim -- --config ./config/${MODE}/client-config.yaml
```

Or, using the container image (assuming the image name is
`slim`):

```bash
docker run -it \
    -e PASSWORD=${PASSWORD} \
    -v ./config/base/client-config.yaml:/config.yaml \
    slim /slim --config /config.yaml
```
