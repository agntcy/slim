# Changelog - AGNTCY Slim

All notable changes to this project will be documented in this file.

## [Unreleased]

## v0.6.0 (9 October 2025)

### Key Highlights

#### 🎯 Major Features Added

- **Control Plane Group Management**: Complete implementation of group management functionality for organizing and controlling SLIM node groups
- **Enhanced Authentication**: In the data-plane, replaced bearer authentication with static JWT tokens. In the control-plane, added basic authentication support for slimctl and k8s ingress in helm chart
- **Session API Refactoring**: Major improvements to session handling including metadata support, enhanced point-to-point sessions with sender/receiver buffers, and removal of request-reply API
- **Python Bindings Improvements**: Unique SLIM data-plane instance per process by default, improved publish function, better documentation, and session type exposure
- **Node Management**: Enhanced node update handling and improved group ID integration in node identification

#### 📜 Documentation

- Enhanced Python bindings documentation with comprehensive session examples
- Updated group example documentation and usage templates
- Improved README files for Python bindings examples
- Added deployment usage templates and Taskfile automation for various deployment patterns ([#753](https://github.com/agntcy/slim/pull/753))

#### ⚠ Breaking Changes

- Session API refactoring with new receive() API pattern
- Removal of anycast session type, renaming to PointToPoint and Group sessions
- Removal of request-reply API from Python bindings
- Authentication method changes (bearer auth → static JWT)
- Group configuration changes (removed moderator parameter)
- Python bindings: removed destination_name property

#### 🔧 Infrastructure & Tooling

- Upgraded to Rust toolchain 1.90.0
- Enhanced release process with signoff commits and pre-release support
- Improved CI/CD with release-please integration
- Better dependency management for Python packages

#### 🛡 Security & Hardening

- Improved certificate handling across trust domains
- Enhanced JWKS file management for multiple trust domains
- Basic authentication support for ingress and slimctl

### Component Versions Summary

| Component               | Latest Version | Release Date |
| ----------------------- | -------------- | ------------ |
| slim                    | v0.6.0         | 2025-10-09   |
| slim-bindings           | v0.6.0         | 2025-10-09   |
| control-plane           | v0.6.0         | 2025-10-09   |
| slimctl                 | v0.6.0         | 2025-10-09   |
| slim-bindings-examples  | v0.6.0         | 2025-10-09   |
| slim-testutils          | v0.6.0         | 2025-10-09   |
| agntcy-slim-mcp-proxy   | v0.2.0         | 2025-10-09   |

### Release Artifacts

- **Container Images**: Available on GitHub Container Registry
  - `ghcr.io/agntcy/slim:v0.6.0`
  - `ghcr.io/agntcy/slim-control-plane:v0.6.0`
- **Python Packages**: Published to PyPI
  - `slim-bindings==0.6.0`
- **Helm Charts**: Available on Helm repository
  - `helm-slim` (updated for v0.6.0 compatibility)
  - `helm-slim-control-plane` (updated for control-plane v0.6.0)
- **CLI Tools**:
  - `slimctl` v0.6.0 with enhanced authentication support

### Compatibility Matrix

The following matrix shows compatibility between different component versions:

| Core Component    | Version | Compatible With                                             |
| ----------------- | ------- | ----------------------------------------------------------- |
| **slim**          | v0.6.0  | slim-bindings v0.6.0, control-plane v0.6.0                 |
| **slim-bindings** | v0.6.0  | slim v0.6.0                                                 |
| **control-plane** | v0.6.0  | slim v0.6.0, slimctl v0.6.0                                |
| **slimctl**       | v0.6.0  | control-plane v0.6.0, slim v0.6.0                          |

#### Helm Chart Compatibility

| Helm Chart                  | Version | Deploys Component     | Minimum Requirements |
| --------------------------- | ------- | --------------------- | -------------------- |
| **helm-slim**               | v0.6.0  | slim v0.6.0           | Kubernetes 1.20+     |
| **helm-slim-control-plane** | v0.6.0  | control-plane v0.6.0  | Kubernetes 1.20+     |

#### Development & Testing Tools

| Tool                       | Version | Works With           | Purpose           |
| -------------------------- | ------- | -------------------- | ----------------- |
| **slim-testutils**         | v0.6.0  | All v0.6.0 components| Testing utilities |
| **slim-bindings-examples** | v0.6.0  | slim-bindings v0.6.0 | Python examples   |

#### Compatibility Notes

- **slim v0.6.0** introduces breaking changes in session API that require **slim-bindings v0.6.0**
- **Group management** requires **control-plane v0.6.0** and **slimctl v0.6.0**
- **Enhanced authentication** is available across all v0.6.0 components
- **Session metadata** and **improved session handling** require slim v0.6.0 and slim-bindings v0.6.0
- Older versions of slim-bindings (< v0.6.0) are **not compatible** with slim v0.6.0 due to session API changes

### v0.6.0 Release Summary (October 2025)

#### ⚠ Breaking Changes

- Session API refactoring with new receive() pattern ([#731](https://github.com/agntcy/slim/pull/731))
- Remove anycast session, rename other sessions into PointToPoint and Group ([#795](https://github.com/agntcy/slim/pull/795))
- Remove request-reply API from Python bindings ([#677](https://github.com/agntcy/slim/pull/677))
- Remove bearer auth in favour of static JWT ([#774](https://github.com/agntcy/slim/pull/774))
- Remove moderator parameter from Group configuration ([#739](https://github.com/agntcy/slim/pull/739))
- Remove destination_name property from Python bindings ([#751](https://github.com/agntcy/slim/pull/751))

#### 🚀 Features

- Implement control plane group management ([#554](https://github.com/agntcy/slim/pull/554))
- Handle updates from SLIM nodes ([#708](https://github.com/agntcy/slim/pull/708))
- Create unique SLIM data-plane instance per process by default ([#819](https://github.com/agntcy/slim/pull/819))
- Introduce session metadata ([#744](https://github.com/agntcy/slim/pull/744))
- Expose session type, src and dst names in Python sessions ([#737](https://github.com/agntcy/slim/pull/737))
- Improve point to point session with sender/receiver buffer ([#735](https://github.com/agntcy/slim/pull/735))
- Improve publish function in Python bindings ([#749](https://github.com/agntcy/slim/pull/749))
- Add basic auth to slimctl ([#763](https://github.com/agntcy/slim/pull/763))
- Add basic auth to ingress ([#722](https://github.com/agntcy/slim/pull/722))
- Add string name on pub messages ([#693](https://github.com/agntcy/slim/pull/693))
- Allow each participant to publish in Python examples ([#778](https://github.com/agntcy/slim/pull/778))
- Improve Python bindings documentation ([#748](https://github.com/agntcy/slim/pull/748))
- Add documentation for sessions and examples ([#750](https://github.com/agntcy/slim/pull/750))

#### 🐛 Bug Fixes

- Fix subscription-table wrong iterator when matching over multiple output connections ([#815](https://github.com/agntcy/slim/pull/815))
- Avoid panic sending errors to the local application ([#814](https://github.com/agntcy/slim/pull/814))
- Add group id to node id ([#746](https://github.com/agntcy/slim/pull/746))
- Create new JWKS file containing all keys from all trust domains ([#776](https://github.com/agntcy/slim/pull/776))
- Load all certificates for dataplane from ca ([#772](https://github.com/agntcy/slim/pull/772))
- Correctly close group example ([#786](https://github.com/agntcy/slim/pull/786))
- Fix readmes for python bindings examples ([#764](https://github.com/agntcy/slim/pull/764))

#### 🔧 Infrastructure & Tooling

- Upgrade to rust toolchain 1.90.0 ([#730](https://github.com/agntcy/slim/pull/730))
- Signoff commits made by release please ([#723](https://github.com/agntcy/slim/pull/723))
- Fix release please signoff ([#727](https://github.com/agntcy/slim/pull/727))
- Fix python examples path in CI ([#719](https://github.com/agntcy/slim/pull/719))
- Publish Python bindings as pre-release ([#787](https://github.com/agntcy/slim/pull/787), [#793](https://github.com/agntcy/slim/pull/793))

#### 🛡 Security & Hardening

- Enhanced certificate management across trust domains ([#776](https://github.com/agntcy/slim/pull/776))
- Static JWT authentication implementation ([#774](https://github.com/agntcy/slim/pull/774))
- Basic authentication for slimctl and ingress ([#763](https://github.com/agntcy/slim/pull/763), [#722](https://github.com/agntcy/slim/pull/722))

#### 📦 Packaging & Release

- Coordinated multi-component release (slim 0.6.0 & dependent packages)
- Remove -rc suffix from python bindings ([#821](https://github.com/agntcy/slim/pull/821))
- Update bindings dependencies across packages ([#713](https://github.com/agntcy/slim/pull/713))
- Upgrade examples to use slim 0.5.0+ ([#717](https://github.com/agntcy/slim/pull/717))

### v0.6.0 Session API migration guide

The API has undergone a fundamental architectural shift from **app-centric messaging** to **session-centric messaging**:

- **Old paradigm**: Messages sent through the `Slim` application instance with session IDs
- **New paradigm**: Messages sent directly through `Session` objects

---

### 1. Dependency Updates

#### Update your requirements
```diff
# pyproject.toml or requirements.txt
- slim-bindings>=0.5.0
+ slim-bindings>=0.6.0
```

---

### 2. Application Instance Management

#### Instance ID Access
```python
# OLD: Method call
app = await slim_bindings.Slim.new(name, provider, verifier)
instance_id = app.get_id()

# NEW: Property access
app = await slim_bindings.Slim.new(name, provider, verifier)
instance_id_str = app.id_str
instance_id = app.id
```

#### Context Manager Usage
```python
# OLD: Required context manager
async with app:
    session = await app.create_session(config)
    await app.publish(session, message, destination)

# NEW: No context manager needed
session = await app.create_session(config)
await session.publish(message)
```

**Migration**: Remove `async with app:` context managers, replace `app.get_id()` with `app.id_str` or `app.id`.

---

### 3. Session Configuration (Major Changes)

The session configuration system has been completely redesigned around communication patterns.

#### Old Session Types → New Session Types

##### Fire-and-Forget → PointToPoint

```python
# OLD: Fire-and-forget with sticky=True
session = await app.create_session(
    slim_bindings.PySessionConfiguration.FireAndForget(
        max_retries=5,
        timeout=datetime.timedelta(seconds=5),
        sticky=True,  # Sticky = same peer
        mls_enabled=enable_mls,
    )
)

# NEW: Explicit PointToPoint
session = await app.create_session(
    slim_bindings.PySessionConfiguration.PointToPoint(
        peer_name=remote_name,
        # uncomment to enable reliable delivery and MLS
        # max_retries=5,
        # timeout=datetime.timedelta(seconds=5),
        # mls_enabled=enable_mls,
    )
)
```

##### Streaming → Group

```python
# OLD: Bidirectional streaming with topic
session = await app.create_session(
    slim_bindings.PySessionConfiguration.Streaming(
        slim_bindings.PySessionDirection.BIDIRECTIONAL,
        topic=channel_name,
        moderator=True,
        max_retries=5,
        timeout=datetime.timedelta(seconds=5),
        mls_enabled=enable_mls,
    )
)

# NEW: Group with channel
session = await app.create_session(
    slim_bindings.PySessionConfiguration.Group(
        channel_name=channel_name,
        # uncomment to enable reliable delivery and MLS
        # max_retries=5,
        # timeout=datetime.timedelta(seconds=5),
        # mls_enabled=enable_mls,
    )
)
```

**Migration Mapping**:
- `FireAndForget(sticky=True)` → `PointToPoint(peer_name=target)`
- `Streaming(BIDIRECTIONAL, topic=X)` → `Group(channel_name=target_group)`

---

#### 5. Message Sending

The message sending API has moved from app-level methods to session-level methods.

##### Basic Message Publishing
```python
# OLD: App-level publishing with session reference
await app.publish(session, message.encode(), destination_name)

# NEW: Session-level publishing

# For PointToPoint/Group (destination implicit)
await session.publish(message.encode())
```

##### Reply to Received Messages
```python
# OLD: App-level reply
await app.publish_to(session_info, reply.encode())

# NEW: Session-level reply with message context
await session.publish_to(msg_ctx, reply.encode())
```

---

#### 6. Message Receiving

Message receiving has changed from app-level session management to session-level message handling.

##### Waiting for New Sessions
```python
# OLD: App-level receive for new sessions
session_info, initial_message = await app.receive()
session_id = session_info.id

# NEW: Explicit session listening
session = await app.listen_for_session()

# Session also contains additional properties
session_id = session.id
session_metadata = session.metadata
session_type = session.session_type
session_config = session.session_config
session_src = session.src
session_dst = session.dst
```

##### Receiving Messages from Sessions
```python
# OLD: App-level receive with session ID
session_info, message = await app.receive(session=session_id)

# NEW: Session-level message receiving
msg_ctx, message = await session.get_message()
```

##### Message Context Changes
```python
# OLD: Session info contained properties regarding the last message received
source = session_info.destination_name:
dst = session_info.source_name

# NEW: Message context contains additional received message properties
source_name = msg_ctx.source_name
dst_name = msg_ctx.destination_name
payload_type = msg_ctx.payload_type
message_metadata = msg_ctx.metadata
input_connection = msg_ctx.input_connection
```

---

#### 7. Session Lifecycle Management

##### Session Object Usage
```python
# OLD: Sessions referenced by ID, managed through app
async def handle_session(app, session_id):
    while True:
        session_info, msg = await app.receive(session=session_id)
        await app.publish_to(session_info, response.encode())

# NEW: Sessions are first-class objects
async def handle_session(session):
    while True:
        msg_ctx, msg = await session.get_message()
        await session.publish_to(msg_ctx, response.encode())
```

##### Multiple Session Handling
```python
# OLD: Track sessions by ID
active_sessions = {}
session_info, _ = await app.receive()
session_id = session_info.id
active_sessions[session_id] = session_info

# NEW: Work directly with session objects
active_sessions = []
session = await app.listen_for_session()
active_sessions.append(session)
```

---

#### 8. Group Management (Invitations)

##### Inviting Participants
```python
# OLD: App-level invitation management
await app.invite(session_info, participant_name)

# NEW: Session-level invitation management

# Add a participant
await session.invite(participant_name)

# Remove a participant
await session.remove(participant_name)
```

---

#### 9. Error Handling Changes

##### Session-Related Errors
```python
# OLD: Errors handled at app level with session IDs
try:
    session_info, msg = await app.receive(session=session_id)
except Exception as e:
    # Handle session-specific errors

# NEW: Errors handled at session level
try:
    msg_ctx, msg = await session.get_message()
except Exception as e:
    # Handle session-specific errors
```

---

#### 10. Complete Migration Checklist

##### Phase 1: Dependencies and Basic Setup
- [ ] Update `slim-bindings` to `>=0.6.0`
- [ ] Replace `app.get_id()` with `app.id_str`
- [ ] Remove `async with app:` context managers

##### Phase 2: Session Configuration
- [ ] Map `FireAndForget(sticky=True)` → `PointToPoint(peer_name=target)`
- [ ] Map `Streaming(BIDIRECTIONAL)` → `group(channel_name=topic)`
- [ ] Update session configuration parameters

##### Phase 3: Message Sending
- [ ] Replace `app.publish()` with `session.publish()`
- [ ] Replace `app.publish_to()` with `session.publish_to()`

##### Phase 4: Message Receiving
- [ ] Replace `app.receive()` with `app.listen_for_session()` for new sessions
- [ ] Replace `app.receive(session=id)` with `session.get_message()`
- [ ] Update message context handling (`msg_ctx.source_name` vs `session_info.destination_name`)

##### Phase 5: Session Management
- [ ] Update background task signatures to accept session objects instead of IDs
- [ ] Replace `app.invite()` with `session.invite()`
- [ ] Update session tracking to use objects instead of IDs

##### Phase 6: Testing and Validation
- [ ] Test all communication patterns (PointToPoint, Group)
- [ ] Verify MLS functionality if used
- [ ] Test error handling and edge cases
- [ ] Validate performance characteristics

---

## v0.5.0 (19 September 2025)

### Key Highlights

#### 🎯 Major Features Added

- SLIMRPC + Native A2A integration (Python) with new `slimrpc` and `slima2a`
  packages
- Automatic generation of SLIMRPC client stubs and server servicers for python
  via the `protoc-slimrpc-plugin` protoc plugin
- HTTP / HTTPS proxy support for gRPC clients
- Node ID uniqueness & enhanced deployment options (StatefulSet / DaemonSet)
- Control plane test coverage & Ack original message ID validation
- SPIRE & TLS configuration support in Controller Southbound API & Helm charts

#### 📜 Documentation

- Documentation updates across components:
  - [SLIMRPC](https://docs.agntcy.org/messaging/slim-rpc/)
  - [SLIMA2A](https://docs.agntcy.org/messaging/slim-a2a/)
  - [PROTOC-SLIMRPC-PLUGIN](https://docs.agntcy.org/messaging/slim-slimrpc-compiler/)
  - [Configuration Reference](https://docs.agntcy.org/messaging/slim-data-plane-config/)

#### ⚠ Breaking Changes

- Node ID uniqueness requirement (Helm & service config)
- Multiple Rust public API changes in dataplane release (v0.5.0 bundle)
- Service publish API parameter expansions

#### 🔧 Infrastructure & Tooling

- Clang-19 migration for data-plane builds of C components
- Conventional commit PR title enforcement
- Centralized Python integrations layout (`data-plane/python/...`)
- Coverage reporting for Rust & Control Plane

#### 🛡 Security & Hardening

- Default system CA loading in TLS config
- Dependency upgrades (tracing-subscriber, slab)

---

_For detailed component histories see: [slim](./data-plane/core/README.md),
[slim data-plane crates](./data-plane/), [python
bindings](./data-plane/python/bindings/CHANGELOG.md), [python integrations
slimrpc](./data-plane/python/integrations/slimrpc/CHANGELOG.md),
[slima2a](./data-plane/python/integrations/slima2a/CHANGELOG.md),
[slim-mcp](./data-plane/python/integrations/slim-mcp/CHANGELOG.md),
[control-plane](./control-plane/control-plane/CHANGELOG.md),
[slimctl](./control-plane/slimctl/CHANGELOG.md),
[helm-slim](./charts/slim/CHANGELOG.md),
[helm-slim-control-plane](./charts/slim-control-plane/CHANGELOG.md)._

### Component Versions Summary

| Component                                                           | Latest Version | Release Date |
| ------------------------------------------------------------------- | -------------- | ------------ |
| [slim](./data-plane/)                                               | v0.5.0         | 2025-09-19   |
| [slim-bindings](./data-plane/python/bindings/CHANGELOG.md)          | v0.5.0         | 2025-09-19   |
| [control-plane](./control-plane/control-plane/CHANGELOG.md)         | v0.1.1         | 2025-09-19   |
| [slimctl](./control-plane/slimctl/CHANGELOG.md)                     | v0.2.2         | 2025-09-19   |
| [helm-slim](./charts/slim/CHANGELOG.md)                             | v0.2.0         | 2025-09-19   |
| [helm-slim-control-plane](./charts/slim-control-plane/CHANGELOG.md) | v0.1.4         | 2025-09-19   |
| [slim-bindings-examples](./data-plane/python/bindings/examples/)    | v0.1.1         | 2025-09-19   |
| [slim-testutils](./data-plane/testing/CHANGELOG.md)                 | v0.2.2         | 2025-09-19   |
| [slimrpc](./data-plane/python/integrations/slimrpc/CHANGELOG.md)    | v0.1.0         | 2025-09-19   |
| [slima2a](./data-plane/python/integrations/slima2a/CHANGELOG.md)    | v0.1.0         | 2025-09-19   |
| [slim-mcp](./data-plane/python/integrations/slim-mcp/CHANGELOG.md)  | v0.1.7         | 2025-09-19   |
| [slim-mcp-proxy](./data-plane/integrations/mcp-proxy/CHANGELOG.md)  | v0.1.7         | 2025-09-19   |

### Release Artifacts

| Component               | Tag                              | Artifacts                                                                      |
| ----------------------- | -------------------------------- | ------------------------------------------------------------------------------ |
| slim                    | `slim-v0.5.0`                    | https://github.com/agntcy/slim/pkgs/container/slim                             |
| slim-bindings           | `slim-bindings-v0.5.0`           | https://pypi.org/project/slim-bindings                                         |
| control-plane           | `control-plane-v0.1.1`           | https://github.com/agntcy/slim/pkgs/container/slim%2Fcontrol-plane             |
| slimctl                 | `slimctl-v0.2.2`                 | https://github.com/agntcy/slim/releases/tag/slimctl-v0.2.2                     |
| helm-slim               | `helm-slim-v0.2.0`               | https://github.com/agntcy/slim/pkgs/container/slim%2Fhelm%2Fslim               |
| helm-slim-control-plane | `helm-slim-control-plane-v0.1.4` | https://github.com/agntcy/slim/pkgs/container/slim%2Fhelm%2Fslim-control-plane |
| slim-bindings-examples  | `slim-bindings-examples-v0.1.1`  | https://github.com/agntcy/slim/pkgs/container/slim%2Fbindings-examples         |
| slim-testutils          | `slim-testutils-v0.2.2`          | https://github.com/agntcy/slim/pkgs/container/slim%2Ftestutils                 |
| slimrpc                 | `slimrpc-v0.1.0`                 | https://pypi.org/project/slimrpc                                               |
| slima2a                 | `slima2a-v0.1.0`                 | https://pypi.org/project/slima2a                                               |
| slim-mcp                | `slim-mcp-v0.1.7`                | https://pypi.org/project/slim-mcp                                              |
| slim-mcp-proxy          | `slim-mcp-proxy-v0.1.7`          | https://github.com/agntcy/slim/pkgs/container/slim%2Fmcp-proxy                 |
| protoc-slimrpc-plugin   | `protoc-slimrpc-plugin-v0.1.0`   | https://crates.io/crates/agntcy-protoc-slimrpc-plugin                          |

### Compatibility Matrix

| Core Component    | Version | Compatible With                                             |
| ----------------- | ------- | ----------------------------------------------------------- |
| **slim**          | v0.5.0  | control-plane v0.1.1                                        |
| **slim-bindings** | v0.5.0  | slim v0.5.0                                                 |
| **control-plane** | v0.1.1  | slim v0.5.0, slimctl v0.2.2, helm-slim-control-plane v0.1.4 |
| **slimctl**       | v0.2.2  | control-plane v0.1.1, slim v0.5.0                           |
| **slimrpc**       | v0.1.0  | slim-bindings v0.5.0                                        |
| **slima2a**       | v0.1.0  | slimrpc v0.1.0, slim-bindings v0.5.0                        |

#### Helm Chart Compatibility

| Helm Chart                  | Version | Deploys Component    | Minimum Requirements |
| --------------------------- | ------- | -------------------- | -------------------- |
| **helm-slim**               | v0.2.0  | slim v0.5.0          | Kubernetes 1.20+     |
| **helm-slim-control-plane** | v0.1.4  | control-plane v0.1.1 | Kubernetes 1.20+     |

#### Development & Testing Tools

| Tool                       | Version | Works With     | Purpose           |
| -------------------------- | ------- | -------------- | ----------------- |
| **slim-testutils**         | v0.2.2  | slim >= v0.5.0 | Testing utilities |
| **slim-bindings-examples** | v0.1.1  | slim >= v0.4.0 | Python examples   |

#### Compatibility Notes

- Node ID uniqueness introduces deployment config updates in Helm and service
  config
- New proxy fields & metadata maps require regenerating client/server config
  initializations
- Service publish API parameter changes may require adjusting function calls

---

### v0.5.0 Release Summary (September 2025)

#### ⚠ Breaking Changes

- Node ID uniqueness & service/Helm config schema updates
  ([#630](https://github.com/agntcy/slim/pull/630))
- Data-plane public API adjustments across
  auth/config/datapath/controller/service crates (bundle release)
  ([#508](https://github.com/agntcy/slim/pull/508))

#### 🚀 Features

- Subscription notifications propagated to controller & services
  ([#611](https://github.com/agntcy/slim/pull/611))
- Metadata map for gRPC clients/servers
  ([#684](https://github.com/agntcy/slim/pull/684))
- Replace pubsub with dataplane in node-config
  ([#591](https://github.com/agntcy/slim/pull/591))
- Ack original messageID validation
  ([#583](https://github.com/agntcy/slim/pull/583))
- Control-plane test coverage + combined coverage workflow
  ([#664](https://github.com/agntcy/slim/pull/664))
- MLS identity provider backend agnostic
  ([#552](https://github.com/agntcy/slim/pull/552))
- Fire-and-forget session testing utilities
  ([#540](https://github.com/agntcy/slim/pull/540))
- Channel & participant CLI commands (slimctl)
  ([#534](https://github.com/agntcy/slim/pull/534))
- Channel info included in subscription messages
  ([#611](https://github.com/agntcy/slim/pull/611))
- SLIMRPC + Native A2A Python packages (`slimrpc`, `slima2a`) initial release
  ([#660](https://github.com/agntcy/slim/pull/660)
  [#685](https://github.com/agntcy/slim/pull/685)
  [#690](https://github.com/agntcy/slim/pull/690)
  [#613](https://github.com/agntcy/slim/pull/613))
- HTTP & HTTPS proxy support for gRPC client
  ([#610](https://github.com/agntcy/slim/pull/610)
  [#614](https://github.com/agntcy/slim/pull/614))
- TLS & SPIRE config support in Controller SB API / Helm
  ([#651](https://github.com/agntcy/slim/pull/651))
- Node ID based StatefulSet / DaemonSet deployment options
  ([#630](https://github.com/agntcy/slim/pull/630))
- First release of Slim RPC protoc compiler plugin (`protoc-slimrpc-plugin`)
  enabling Slim RPC code generation (tag `protoc-slimrpc-plugin-v0.1.0`)

#### 🐛 Bug Fixes

- Duration string normalization (`duration-str` → `duration-string`)
  ([#683](https://github.com/agntcy/slim/pull/683))
- Fire-and-forget session reliability
  ([#538](https://github.com/agntcy/slim/pull/538))
- WaitGroup for gRPC server startup ordering
  ([#675](https://github.com/agntcy/slim/pull/675))
- Host & port fields in NodeEntry
  ([#560](https://github.com/agntcy/slim/pull/560))
- System CA loading enabled by default in TLS config
  ([#605](https://github.com/agntcy/slim/pull/605))
- Ingress port reference correction (Helm)
  ([#597](https://github.com/agntcy/slim/pull/597))
- Crypto provider initialization for Reqwest (Python bindings)
  ([#706](https://github.com/agntcy/slim/pull/706))
- Mypy hygiene across python packages (slima2a, slimrpc, slim-mcp)
  ([#671](https://github.com/agntcy/slim/pull/671))
- Slimctl version crash resolved
  ([#585](https://github.com/agntcy/slim/pull/585))
- Deadline handling in SLIMRPC ([#658](https://github.com/agntcy/slim/pull/658))
- Template comment usage to avoid Helm rendering issues
  ([#595](https://github.com/agntcy/slim/pull/595))

#### 🔧 Infrastructure & Tooling

- Move to clang-19 for data-plane C/C++ builds
  ([#662](https://github.com/agntcy/slim/pull/662))
- PR title conventional commit enforcement
  ([#686](https://github.com/agntcy/slim/pull/686))
- Python integrations folder restructure & Taskfile division
  ([#628](https://github.com/agntcy/slim/pull/628))
- Coverage reporting (Rust + Control Plane)
  ([#652](https://github.com/agntcy/slim/pull/652)
  [#664](https://github.com/agntcy/slim/pull/664))
- Buf CI config update ([#532](https://github.com/agntcy/slim/pull/532))

#### 🛡 Security & Hardening

- Default system CA trust configuration
  ([#605](https://github.com/agntcy/slim/pull/605))
- Dependency upgrades (tracing-subscriber 0.3.20, slab)
  ([#608](https://github.com/agntcy/slim/pull/608)
  [#563](https://github.com/agntcy/slim/pull/563))

#### 📦 Packaging & Release

- Coordinated multi-component release (slim 0.5.0 & dependent charts/bindings)
- Image upgrades in Helm charts (slim 0.5.0, control-plane 0.1.1)
  ([#714](https://github.com/agntcy/slim/pull/714)
  [#716](https://github.com/agntcy/slim/pull/716))
- Python wheels published for new packages (slimrpc, slima2a)
  ([#638](https://github.com/agntcy/slim/pull/638)
  [#639](https://github.com/agntcy/slim/pull/639)
  [#637](https://github.com/agntcy/slim/pull/637))

---

## v0.4.0 (August 2025)

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

_For detailed information about specific releases, see the individual component
CHANGELOG.md files in their respective directories._

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

- **slim v0.4.0** introduces breaking changes that require **slim-bindings
  v0.4.0** or later
- **MLS integration** requires **slim v0.4.0** and **slim-bindings v0.4.0**
- **Control plane features** require **control-plane v0.1.0** + **slimctl
  v0.2.1**
- **Authentication support** is available across **slim v0.4.0**,
  **slim-bindings v0.4.0**, and **control-plane v0.1.0**
- Older versions of slim-bindings (< v0.4.0) are **not compatible** with slim
  v0.4.0 due to breaking changes

### [slim-v0.4.0] - 2025-07-31

**Core Slim Data Plane**

#### ⚠ BREAKING CHANGES

- remove Agent and AgentType and adopt Name as application identifier
  ([#477](https://github.com/agntcy/slim/pull/477))

#### 🚀 Features

- implement MLS key rotation ([#412](https://github.com/agntcy/slim/issues/412))
- improve handling of commit broadcast
  ([#433](https://github.com/agntcy/slim/issues/433))
- implement key rotation proposal message exchange
  ([#434](https://github.com/agntcy/slim/issues/434))
- **auth:** support JWK as decoding keys
  ([#461](https://github.com/agntcy/slim/issues/461))
- integrate MLS with auth ([#385](https://github.com/agntcy/slim/issues/385))
- add mls message types in slim messages
  ([#386](https://github.com/agntcy/slim/issues/386))
- push and verify identities in message headers
  ([#384](https://github.com/agntcy/slim/issues/384))
- channel creation in session layer
  ([#374](https://github.com/agntcy/slim/issues/374))

#### 🐛 Bug Fixes

- prevent message publication before mls setup
  ([#458](https://github.com/agntcy/slim/issues/458))
- remove all state on session close
  ([#449](https://github.com/agntcy/slim/issues/449))
- **channel_endpoint:** extend mls for all sessions
  ([#411](https://github.com/agntcy/slim/issues/411))
- remove request-reply session type
  ([#416](https://github.com/agntcy/slim/issues/416))
- **auth:** make simple identity usable for groups
  ([#387](https://github.com/agntcy/slim/issues/387))

### [slim-bindings-v0.4.0] - 2025-07-31

**Python Bindings**

#### ⚠ BREAKING CHANGES

- remove request-reply session type
  ([#416](https://github.com/agntcy/slim/issues/416))

#### 🚀 Features

- add auth support in sessions
  ([#382](https://github.com/agntcy/slim/issues/382))
- get source and destination name from python
  ([#485](https://github.com/agntcy/slim/issues/485))
- push and verify identities in message headers
  ([#384](https://github.com/agntcy/slim/issues/384))
- add identity and mls options to python bindings
  ([#436](https://github.com/agntcy/slim/issues/436))
- **python-bindings:** update examples and make them packageable
  ([#468](https://github.com/agntcy/slim/issues/468))

### [control-plane-v0.1.0] - 2025-07-31

**Control Plane Service**

#### 🚀 Features

- add api endpoints for group management
  ([#450](https://github.com/agntcy/slim/issues/450))
- control plane service & slimctl cp commands
  ([#388](https://github.com/agntcy/slim/issues/388))
- group svc backend with inmem db
  ([#456](https://github.com/agntcy/slim/issues/456))
- process concurrent modification to the group
  ([#451](https://github.com/agntcy/slim/issues/451))

#### 🐛 Bug Fixes

- control plane config is not loaded
  ([#452](https://github.com/agntcy/slim/issues/452))

### [slimctl-v0.2.1] - 2025-08-01

**Slim Control CLI Tool**

#### 🚀 Features

- add client connections to control plane
  ([#429](https://github.com/agntcy/slim/issues/429))
- add node register call to proto
  ([#406](https://github.com/agntcy/slim/issues/406))
- control plane service & slimctl cp commands
  ([#388](https://github.com/agntcy/slim/issues/388))
- **control-plane:** handle all configuration parameters when creating a new
  connection ([#360](https://github.com/agntcy/slim/issues/360))

#### 🐛 Bug Fixes

- slimctl: add scheme to endpoint param
  ([#459](https://github.com/agntcy/slim/issues/459))
- **control-plane:** always run go generate
  ([#496](https://github.com/agntcy/slim/issues/496))

### [helm-slim-v0.1.9] - 2025-07-31

**Main Helm Chart**

#### 🚀 Features

- **slim-helm:** upgrade helm to slim image 0.4.0
  ([#495](https://github.com/agntcy/slim/issues/495))

#### 🐛 Bug Fixes

- add slim.overrideConfig to helm values
  ([#490](https://github.com/agntcy/slim/issues/490))

### [helm-slim-control-plane-v0.1.3] - 2025-08-01

**Helm Chart for Slim Control Plane**

#### 🚀 Features

- **slim-control-plane:** upgrade chart to latest control-plane image
  ([#503](https://github.com/agntcy/slim/issues/503))

#### 🐛 Bug Fixes

- **control-plane:** run go generate when running linter
  ([#505](https://github.com/agntcy/slim/issues/505))

### [slim-bindings-examples-v0.1.0] - 2025-08-01

**Python Bindings Examples Package**

#### 🚀 Features

- **data-plane/service:** first draft of session layer
  ([#106](https://github.com/agntcy/slim/issues/106))
- get source and destination name from python
  ([#485](https://github.com/agntcy/slim/issues/485))
- **python-bindings:** update examples and make them packageable
  ([#468](https://github.com/agntcy/slim/issues/468))
- improve configuration handling for tracing
  ([#186](https://github.com/agntcy/slim/issues/186))
- **session:** add default config for sessions created upon message reception
  ([#181](https://github.com/agntcy/slim/issues/181))

#### 🐛 Bug Fixes

- **python-bindings:** fix python examples
  ([#120](https://github.com/agntcy/slim/issues/120))
- **python-bindings:** fix examples and taskfile
  ([#340](https://github.com/agntcy/slim/issues/340))

### [slim-testutils-v0.2.1] - 2025-08-01

**Testing Utilities**

#### 🐛 Bug Fixes

- **testutils:** use common dockerfile to build testutils
  ([#499](https://github.com/agntcy/slim/issues/499))

### [slim-mls-v0.1.0] - 2025-07-31

**Message Layer Security Implementation**

#### 🚀 Features

- add identity and mls options to python bindings
  ([#436](https://github.com/agntcy/slim/pull/436))
- integrate MLS with auth ([#385](https://github.com/agntcy/slim/pull/385))
- add mls message types in slim messages
  ([#386](https://github.com/agntcy/slim/pull/386))
- push and verify identities in message headers
  ([#384](https://github.com/agntcy/slim/pull/384))
- add the ability to drop messages from the interceptor
  ([#371](https://github.com/agntcy/slim/pull/371))
- implement MLS key rotation ([#412](https://github.com/agntcy/slim/issues/412))
- improve handling of commit broadcast
  ([#433](https://github.com/agntcy/slim/issues/433))
- implement key rotation proposal message exchange
  ([#434](https://github.com/agntcy/slim/issues/434))

## Migration Guide: v0.3.6 to v0.4.0

> ⚠️ **DEPRECATION NOTICE**: SLIM v0.3.6 is now deprecated and will no longer
> receive updates or security patches. Users are strongly encouraged to migrate
> to v0.4.0 or later to benefit from enhanced security features, bug fixes, and
> ongoing support.

This section provides detailed migration instructions for upgrading from SLIM
v0.3.6 to v0.4.0, particularly focusing on the Python bindings changes.

### Breaking Changes Summary

The following breaking changes require code modifications when upgrading:

- **SLIM instance creation method** has changed
- **`PyAgentType` class** has been replaced with the `PyName` class
- **Session creation** is no longer mandatory for non-moderator applications in
  groups
- **Channel invitation process** is now mandatory for establishing secure
  communication
- **`provider` and `verifier` parameters** are now required when creating a SLIM
  instance

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

The `PyAgentType` class has been replaced with the more generic `PyName` class,
reflecting that any application (not just agents) can utilize SLIM.

#### 2. Required Authentication Parameters

Two new required parameters must be provided:

- **`provider`**: Establishes the application's identity within the SLIM
  ecosystem
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

- [ ] **Update SLIM instance creation** to use `PyName` objects and
      authentication parameters
- [ ] **Replace `PyAgentType`** with `PyName` throughout your codebase
- [ ] **Configure authentication** using either shared secrets (development) or
      JWT tokens (production)
- [ ] **Remove manual route/subscription setup** (now handled automatically)
- [ ] **Update session creation** - only required for moderator applications
- [ ] **Implement channel invitation logic** for moderators
- [ ] **Update message reception loops** to handle invitations
- [ ] **Test MLS encryption** functionality if using secure channels

### Additional Resources

- **[SLIM Group
  Tutorial](https://docs.agntcy.org/messaging/slim-group-tutorial/)** - Complete
  step-by-step guide
- **[SPIRE Integration
  Guide](https://docs.agntcy.org/messaging/slim-group/#using-spire-with-slim)**
  - Production-ready authentication setup
- **[Python
  Examples](https://github.com/agntcy/slim/tree/slim-bindings-v0.4.0/data-plane/python-bindings/examples/src/slim_bindings_examples)**
  - Updated code examples
- **[API Documentation](https://docs.agntcy.org/messaging/slim-core/)** -
  Complete API reference

### Future Enhancements

The next release will integrate support for **Agntcy Identity**, allowing users
to create SLIM applications using their agent badge for even more streamlined
authentication.
