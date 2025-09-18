# Changelog - AGNTCY Slim

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [v0.5.0] - 2025-09-17

### Overview

This release introduces significant enhancements across the Python ecosystem (SLIMRPC, A2A), networking (HTTP/HTTPS proxy support), observability & metadata, controller & Helm chart capabilities (TLS/SPIRE, node identity), and build/coverage automation. Several structural & API changes land that may require migrations for early adopters of the Python RPC layer and deployment charts.

### ⚠ Breaking Changes

- slimrpc: Separation of `Channel` and `ChannelFactory` alters object lifecycle expectations (#685)
- Python import rename from `srpc` to `slimrpc` (#660)
- Control-plane Southbound API updates for group CRUD operations (#478)
- SLIM node identity & deployment: introduction of `node_id` and StatefulSet/DaemonSet selection semantics (#630)

### 🚀 Features

- feat(slimrpc): Support existing slim binding in `ChannelFactory` (#690)
- feat(slimrpc): Separate `Channel` and `ChannelFactory` (#685)
- feat: Add metadata map to clients and servers (#684)
- feat: Add control-plane tests and combined coverage workflow (#664)
- feat: SRPC compiler release (#644)
- feat: Rename srpc Python import to slimrpc (#660)
- feat(srpc): Add traceback in SRPC handler error log (#613)
- feat(grpc-client): Add support for HTTPS proxy (#614)
- feat(grpc): Add support for HTTP proxy (#610)
- feat: Notify controller with new subscriptions (#611)
- feat: Replace pubsub with dataplane in node-config (#591)
- feat: Add original messageID check for Ack (#583)
- feat: Make MLS identity provider backend agnostic (#552)
- feat: Add channel and participant commands (#534)
- feat: Update SB API in control-plane to support group CRUD operations (#478)
- feat: Introduce SRPC + native A2A Python integration (#550)
- feat: Add TLS & SPIRE support to Controller southbound API & chart (#651)
- feat: SLIM node ID uniqueness (node_id property + StatefulSet/DaemonSet option) (#630)

### 🐛 Fixes

- fix(python-bindings): Default crypto provider initialization for Reqwest (#706)
- fix: Use duration-string instead of duration-str (#683)
- fix: Add WaitGroup when starting gRPC servers in Controller (#675)
- fix(setup-rust): Save llvm install script outside workspace (#670)
- fix(slimrpc, slima2a): mypy issues (#671)
- fix(srpc): Deadline handling (#658)
- fix: ff session reliability (#538)
- fix(control-plane): Run go generate when running linter (#505)
- fix(control-plane): Always run go generate (#496)
- fix(testutils): Use common Dockerfile to build testutils (#499)
- fix(helm): Incorrect port reference in ingress (#597)
- fix(helm): Template comments for copyright headers (#595)
- fix(tls): Enable loading of system CA certs by default (#605)
- fix: Added host and port in NodeEntry (#560)
- fix(ci): Python examples image build (#633)
- fix(release-rust): mcp-proxy path (#636)
- fix(slimctl): Version command crash (#586)
- fix: Chart lint & values cleanups (various) (#651, #630)

### ♻ Refactors

- refactor(python-bindings): Improve folder structure & Taskfile division (#628)

### 📚 Documentation

- docs: Update SRPC README (#700)
- doc: Comprehensive README for new packages (#640)
- doc(srpc-compiler): Add README (#650)
- docs: Fix code snippets in changelog (#535)
- docs: Changes from v0.3.6 + deprecation notice (#530)
- chore: Update CONTRIBUTING.md (#665)

### ⚙ CI / Build / Tooling

- ci: Add PR title check (Conventional Commits) (#686)
- ci: Publish Rust coverage to Codecov via Taskfile (#652)
- ci: Fix slimrpc & slima2a working directory (#642)
- ci: Fix slimrpc & slima2a tag names (#641)
- ci: Publish slimrpc & slima2a wheels to PyPI (#634)
- ci: Cross-compiler Dockerfile for python bindings examples (#517)
- ci: Build slimctl for multi-OS/arch (#497)
- ci: Bindings-examples Docker image (#506)
- build(data-plane): Move to clang-19 (#662)
- deps: Upgrade tracing-subscriber to 0.3.20 (security) (#608)
- deps: Update slab version to fix vulnerability (#563)

### 📦 Releases & Version Bumps

Automated component releases (for detail see component-specific CHANGELOGs): slimrpc 0.0.1 (#638), slim-bindings 0.4.1 (#544), slim-bindings 0.4.0 (#341), slim-bindings-examples 0.1.0 (#507), slimctl 0.2.1 (#498), slimctl 0.2.0 (#383), control-plane 0.1.0 (#462), slim-testutils 0.2.1 (#500), slim-testutils 0.2.0 (#295), agntcy-slim-mcp-proxy v0.1.6 (#348), helm-slim 0.1.9 (#479), helm-slim-control-plane 0.1.3 (#504), mls v0.1.0 (#493), wheel version bumps (#637).

### 🧹 Chore / Maintenance

- chore(mypy): Explicit ignore strategy for untyped imports (#689)
- chore: Add ChangeLog file & version compatibility matrix (#510)
- task: Add missing `core:test` dependency in mcp-proxy Taskfile (#592)

### 🔐 Security

- tracing-subscriber <0.3.20 vulnerability mitigation (#608)
- slab vulnerability fix (#563)

### 🧪 Testing & Coverage

- Control-plane tests & combined coverage integration (#664)
- Fire-and-forget session tests (#540)
- Rust coverage publishing workflow (#652)

### 🔧 Configuration / Helm

- TLS & SPIRE support integration (#651)
- Node ID & StatefulSet/DaemonSet deployment options (#630)
- Ingress port fix (#597)
- Template comment safety for charts (#595)

### Migration Notes

1. Update imports from `srpc` to `slimrpc` in Python code.
2. Adjust code to use `ChannelFactory` + `Channel` separation; reuse factory for multiple channels.
3. If deploying via Helm, review new `node_id` and deployment mode (StatefulSet/DaemonSet) values.
4. For control-plane integrations, regenerate / update clients for revised group CRUD Southbound APIs.
5. Ensure TLS/SPIRE configuration values are aligned with new chart fields.

### Contributors

Thanks to all contributors: @msardara @micpapal @Tehsmash @sancyx @muscariello @sambetts @amitami2 @dkolonit @zkacsand @janossk and automation bots.

---


## Latest Releases (August 2025)

### Key Highlights

#### 🎯 Major Features Added

- **MLS (Message Layer Security)** implementation with key rotation support
- **Control Plane Service** connection, route and group management APIs
- **Enhanced Python Bindings** with improved examples and packaging
- **Session Layer** with authentication and identity support
- **Helm Charts** for easier deployment

#### 🔧 Infrastructure Improvements

- Multi-platform builds for slimctl (darwin, linux) x (amd64, arm64)
- Improved CI/CD with better Docker image building
- Enhanced testing utilities and integration tests
- Better configuration handling and tracing support

#### 🛡️ Security Enhancements

- JWT authentication with JWK/JWKS support
- Identity verification in message headers
- Secure group management with concurrent modification handling
- Enhanced MLS integration with authentication layer

---

_For detailed information about specific releases, see the individual component CHANGELOG.md files in their respective directories._

### Component Versions Summary

| Component               | Latest Version | Release Date |
| ----------------------- | -------------- | ------------ |
| slim                    | v0.4.0         | 2025-07-31   |
| slim-bindings           | v0.4.0         | 2025-07-31   |
| control-plane           | v0.1.0         | 2025-07-31   |
| slimctl                 | v0.2.1         | 2025-08-01   |
| helm-slim               | v0.1.9         | 2025-07-31   |
| helm-slim-control-plane | v0.1.3         | 2025-08-01   |
| slim-bindings-examples  | v0.1.0         | 2025-08-01   |
| slim-testutils          | v0.2.1         | 2025-08-01   |

### Compatibility Matrix

The following matrix shows compatibility between different component versions:

| Core Component    | Version | Compatible With                                             |
| ----------------- | ------- | ----------------------------------------------------------- |
| **slim**          | v0.4.0  | slim-bindings v0.4.0, control-plane v0.1.0                  |
| **slim-bindings** | v0.4.0  | slim v0.4.0                                                 |
| **control-plane** | v0.1.0  | slim v0.4.0, slimctl v0.2.1, helm-slim-control-plane v0.1.3 |
| **slimctl**       | v0.2.1  | control-plane v0.1.0, slim v0.4.0                           |

#### Helm Chart Compatibility

| Helm Chart                  | Version | Deploys Component    | Minimum Requirements |
| --------------------------- | ------- | -------------------- | -------------------- |
| **helm-slim**               | v0.1.9  | slim v0.4.0          | Kubernetes 1.20+     |
| **helm-slim-control-plane** | v0.1.3  | control-plane v0.1.0 | Kubernetes 1.20+     |

#### Development & Testing Tools

| Tool                       | Version | Works With           | Purpose           |
| -------------------------- | ------- | -------------------- | ----------------- |
| **slim-testutils**         | v0.2.1  | All components       | Testing utilities |
| **slim-bindings-examples** | v0.1.0  | slim-bindings v0.4.0 | Python examples   |

#### Compatibility Notes

- **slim v0.4.0** introduces breaking changes that require **slim-bindings v0.4.0** or later
- **MLS integration** requires **slim v0.4.0** and **slim-bindings v0.4.0**
- **Control plane features** require **control-plane v0.1.0** + **slimctl v0.2.1**
- **Authentication support** is available across **slim v0.4.0**, **slim-bindings v0.4.0**, and **control-plane v0.1.0**
- Older versions of slim-bindings (< v0.4.0) are **not compatible** with slim v0.4.0 due to breaking changes

### [slim-v0.4.0] - 2025-07-31

**Core Slim Data Plane**

#### ⚠ BREAKING CHANGES

- remove Agent and AgentType and adopt Name as application identifier ([#477](https://github.com/agntcy/slim/pull/477))

#### 🚀 Features

- implement MLS key rotation ([#412](https://github.com/agntcy/slim/issues/412))
- improve handling of commit broadcast ([#433](https://github.com/agntcy/slim/issues/433))
- implement key rotation proposal message exchange ([#434](https://github.com/agntcy/slim/issues/434))
- **auth:** support JWK as decoding keys ([#461](https://github.com/agntcy/slim/issues/461))
- integrate MLS with auth ([#385](https://github.com/agntcy/slim/issues/385))
- add mls message types in slim messages ([#386](https://github.com/agntcy/slim/issues/386))
- push and verify identities in message headers ([#384](https://github.com/agntcy/slim/issues/384))
- channel creation in session layer ([#374](https://github.com/agntcy/slim/issues/374))

#### 🐛 Bug Fixes

- prevent message publication before mls setup ([#458](https://github.com/agntcy/slim/issues/458))
- remove all state on session close ([#449](https://github.com/agntcy/slim/issues/449))
- **channel_endpoint:** extend mls for all sessions ([#411](https://github.com/agntcy/slim/issues/411))
- remove request-reply session type ([#416](https://github.com/agntcy/slim/issues/416))
- **auth:** make simple identity usable for groups ([#387](https://github.com/agntcy/slim/issues/387))

### [slim-bindings-v0.4.0] - 2025-07-31

**Python Bindings**

#### ⚠ BREAKING CHANGES

- remove request-reply session type ([#416](https://github.com/agntcy/slim/issues/416))

#### 🚀 Features

- add auth support in sessions ([#382](https://github.com/agntcy/slim/issues/382))
- get source and destination name from python ([#485](https://github.com/agntcy/slim/issues/485))
- push and verify identities in message headers ([#384](https://github.com/agntcy/slim/issues/384))
- add identity and mls options to python bindings ([#436](https://github.com/agntcy/slim/issues/436))
- **python-bindings:** update examples and make them packageable ([#468](https://github.com/agntcy/slim/issues/468))

### [control-plane-v0.1.0] - 2025-07-31

**Control Plane Service**

#### 🚀 Features

- add api endpoints for group management ([#450](https://github.com/agntcy/slim/issues/450))
- control plane service & slimctl cp commands ([#388](https://github.com/agntcy/slim/issues/388))
- group svc backend with inmem db ([#456](https://github.com/agntcy/slim/issues/456))
- process concurrent modification to the group ([#451](https://github.com/agntcy/slim/issues/451))

#### 🐛 Bug Fixes

- control plane config is not loaded ([#452](https://github.com/agntcy/slim/issues/452))

### [slimctl-v0.2.1] - 2025-08-01

**Slim Control CLI Tool**

#### 🚀 Features

- add client connections to control plane ([#429](https://github.com/agntcy/slim/issues/429))
- add node register call to proto ([#406](https://github.com/agntcy/slim/issues/406))
- control plane service & slimctl cp commands ([#388](https://github.com/agntcy/slim/issues/388))
- **control-plane:** handle all configuration parameters when creating a new connection ([#360](https://github.com/agntcy/slim/issues/360))

#### 🐛 Bug Fixes

- slimctl: add scheme to endpoint param ([#459](https://github.com/agntcy/slim/issues/459))
- **control-plane:** always run go generate ([#496](https://github.com/agntcy/slim/issues/496))

### [helm-slim-v0.1.9] - 2025-07-31

**Main Helm Chart**

#### 🚀 Features

- **slim-helm:** upgrade helm to slim image 0.4.0 ([#495](https://github.com/agntcy/slim/issues/495))

#### 🐛 Bug Fixes

- add slim.overrideConfig to helm values ([#490](https://github.com/agntcy/slim/issues/490))

### [helm-slim-control-plane-v0.1.3] - 2025-08-01

**Helm Chart for Slim Control Plane**

#### 🚀 Features

- **slim-control-plane:** upgrade chart to latest control-plane image ([#503](https://github.com/agntcy/slim/issues/503))

#### 🐛 Bug Fixes

- **control-plane:** run go generate when running linter ([#505](https://github.com/agntcy/slim/issues/505))

### [slim-bindings-examples-v0.1.0] - 2025-08-01

**Python Bindings Examples Package**

#### 🚀 Features

- **data-plane/service:** first draft of session layer ([#106](https://github.com/agntcy/slim/issues/106))
- get source and destination name from python ([#485](https://github.com/agntcy/slim/issues/485))
- **python-bindings:** update examples and make them packageable ([#468](https://github.com/agntcy/slim/issues/468))
- improve configuration handling for tracing ([#186](https://github.com/agntcy/slim/issues/186))
- **session:** add default config for sessions created upon message reception ([#181](https://github.com/agntcy/slim/issues/181))

#### 🐛 Bug Fixes

- **python-bindings:** fix python examples ([#120](https://github.com/agntcy/slim/issues/120))
- **python-bindings:** fix examples and taskfile ([#340](https://github.com/agntcy/slim/issues/340))

### [slim-testutils-v0.2.1] - 2025-08-01

**Testing Utilities**

#### 🐛 Bug Fixes

- **testutils:** use common dockerfile to build testutils ([#499](https://github.com/agntcy/slim/issues/499))

### [slim-mls-v0.1.0] - 2025-07-31

**Message Layer Security Implementation**

#### 🚀 Features

- add identity and mls options to python bindings ([#436](https://github.com/agntcy/slim/pull/436))
- integrate MLS with auth ([#385](https://github.com/agntcy/slim/pull/385))
- add mls message types in slim messages ([#386](https://github.com/agntcy/slim/pull/386))
- push and verify identities in message headers ([#384](https://github.com/agntcy/slim/pull/384))
- add the ability to drop messages from the interceptor ([#371](https://github.com/agntcy/slim/pull/371))
- implement MLS key rotation ([#412](https://github.com/agntcy/slim/issues/412))
- improve handling of commit broadcast ([#433](https://github.com/agntcy/slim/issues/433))
- implement key rotation proposal message exchange ([#434](https://github.com/agntcy/slim/issues/434))

## Migration Guide: v0.3.6 to v0.4.0

> ⚠️ **DEPRECATION NOTICE**: SLIM v0.3.6 is now deprecated and will no longer
receive updates or security patches. Users are strongly encouraged to migrate to
v0.4.0 or later to benefit from enhanced security features, bug fixes, and
ongoing support.

This section provides detailed migration instructions for upgrading from SLIM
v0.3.6 to v0.4.0, particularly focusing on the Python bindings changes.

### Breaking Changes Summary

The following breaking changes require code modifications when upgrading:

- **SLIM instance creation method** has changed
- **`PyAgentType` class** has been replaced with the `PyName` class
- **Session creation** is no longer mandatory for non-moderator applications in groups
- **Channel invitation process** is now mandatory for establishing secure communication
- **`provider` and `verifier` parameters** are now required when creating a SLIM instance

### SLIM Instance Creation Changes

**v0.3.6 (Old):**
```python
async def run_client(local_id, remote_id, address, enable_opentelemetry: bool):
    # Split the local IDs into their respective components
    local_organization, local_namespace, local_agent = split_id(local_id)

    # Create SLIM instance by passing the components of the local name
    participant = await slim_bindings.Slim.new(
        local_organization, local_namespace, local_agent
    )
```

**v0.4.0 (New):**
```python
# Create SLIM instance with PyName object and required authentication
slim_app = await Slim.new(local_name, provider, verifier)
```

### Key Changes Explained

#### 1. PyName Class Replacement
The `PyAgentType` class has been replaced with the more generic `PyName` class, reflecting that any application (not just agents) can utilize SLIM.

#### 2. Required Authentication Parameters
Two new required parameters must be provided:
- **`provider`**: Establishes the application's identity within the SLIM ecosystem
- **`verifier`**: Proves the application's identity within the SLIM ecosystem

#### 3. Authentication Methods
Two primary authentication methods are supported:

**Shared Secret Method:**
```python
# Uses a pre-shared secret for authentication
# Note: Not recommended for production environments
provider: PyIdentityProvider = PyIdentityProvider.SharedSecret(
    identity=identity, shared_secret=secret
)
verifier: PyIdentityVerifier = PyIdentityVerifier.SharedSecret(
    identity=identity, shared_secret=secret
)
```

**JWT Token Method (Recommended):**
```python
# Uses JSON Web Token (JWT) with SPIRE
# SPIRE (SPIFFE Runtime Environment) provides secure, scalable identity management
provider = slim_bindings.PyIdentityProvider.StaticJwt(
    path=jwt_path,
)

pykey = slim_bindings.PyKey(
    algorithm=slim_bindings.PyAlgorithm.RS256,
    format=slim_bindings.PyKeyFormat.Jwks,
    key=slim_bindings.PyKeyData.Content(content=spire_jwks.decode("utf-8")),
)

verifier = slim_bindings.PyIdentityVerifier.Jwt(
    public_key=pykey,
    issuer=iss,
    audience=aud,
    subject=sub,
)
```

#### 4. Simplified Session Management

**v0.3.6 (Manual Setup Required):**
```python
# Connect to slim server
_ = await participant.connect({"endpoint": address, "tls": {"insecure": True}})

# set route for the chat
await participant.set_route(remote_organization, remote_namespace, broadcast_topic)

# Subscribe to the producer topic
await participant.subscribe(remote_organization, remote_namespace, broadcast_topic)

# create pubsub session (mandatory for all applications)
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
```

**v0.4.0 (Automatic Management):**
```python
# Connect API remains unchanged
await slim_app.connect({"endpoint": address, "tls": {"insecure": True}})

# Routes and subscriptions are automatically managed by SLIM
# Session creation only required for moderator applications
```

#### 5. Channel Invitation Process (New Requirement)

v0.4.0 introduces a mandatory channel invitation process that enhances security
and enables proper MLS (Message Layer Security) setup:

**Moderator Session Creation:**
```python
# Only moderators need to explicitly create sessions
session_info = await slim_app.create_session(
    SessionConfiguration.Streaming(
        SessionDirection.BIDIRECTIONAL,
        topic=PyName(organization, namespace, topic_name),
        moderator=True,        # New flag
        mls_enabled=True,      # New flag for encryption
        max_retries=5,
        timeout=datetime.timedelta(seconds=5),
    )
)
```

**Invitation Process:**
```python
# Moderator invites participants to join the channel
await slim_app.set_route(participant_name)
await slim_app.invite(session_info, participant_name)
```

#### 6. Message Reception

**v0.3.6:**
```python
async def background_task():
    msg = f"Hello from {local_agent}"
    async with participant:
        while True:
            try:
                # receive message from session
                recv_session, msg_rcv = await participant.receive(
                    session=session_info.id
                )
```

**v0.4.0:**
```python
# Reception loop handles both invitations and messages 
session_info, _ = await local_app.receive()
format_message_print(
    f"{instance} received a new session:",
    f"{session_info.id}",
)
async def background_task(session_id):
    while True:
        # Receive the message from the session
        session, msg = await local_app.receive(session=session_id)
        format_message_print(
            f"{instance}",
            f"received (from session {session_id}): {msg.decode()}",
        )
```

### Migration Checklist

When upgrading from v0.3.6 to v0.4.0, ensure you:

- [ ] **Update SLIM instance creation** to use `PyName` objects and authentication parameters
- [ ] **Replace `PyAgentType`** with `PyName` throughout your codebase
- [ ] **Configure authentication** using either shared secrets (development) or JWT tokens (production)
- [ ] **Remove manual route/subscription setup** (now handled automatically)
- [ ] **Update session creation** - only required for moderator applications
- [ ] **Implement channel invitation logic** for moderators
- [ ] **Update message reception loops** to handle invitations
- [ ] **Test MLS encryption** functionality if using secure channels

### Additional Resources

- **[SLIM Group Tutorial](https://docs.agntcy.org/messaging/slim-group-tutorial/)** - Complete step-by-step guide
- **[SPIRE Integration Guide](https://docs.agntcy.org/messaging/slim-group/#using-spire-with-slim)** - Production-ready authentication setup
- **[Python Examples](https://github.com/agntcy/slim/tree/slim-bindings-v0.4.0/data-plane/python-bindings/examples/src/slim_bindings_examples)** - Updated code examples
- **[API Documentation](https://docs.agntcy.org/messaging/slim-core/)** - Complete API reference

### Future Enhancements

The next release will integrate support for **Agntcy Identity**, allowing users
to create SLIM applications using their agent badge for even more streamlined
authentication.
