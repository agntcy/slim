# Changelog - AGNTCY Slim

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

*For detailed information about specific releases, see the individual component CHANGELOG.md files in their respective directories.*

### Component Versions Summary

| Component | Latest Version | Release Date |
|-----------|----------------|--------------|
| slim | v0.4.0 | 2025-07-31 |
| slim-bindings | v0.4.0 | 2025-07-31 |
| control-plane | v0.1.0 | 2025-07-31 |
| slimctl | v0.2.1 | 2025-08-01 |
| helm-slim | v0.1.9 | 2025-07-31 |
| helm-slim-control-plane | v0.1.3 | 2025-08-01 |
| slim-bindings-examples | v0.1.0 | 2025-08-01 |
| slim-testutils | v0.2.1 | 2025-08-01 |

### Compatibility Matrix

The following matrix shows compatibility between different component versions:

| Core Component | Version | Compatible With |
|----------------|---------|-----------------|
| **slim** | v0.4.0 | slim-bindings v0.4.0, control-plane v0.1.0 |
| **slim-bindings** | v0.4.0 | slim v0.4.0 |
| **control-plane** | v0.1.0 | slim v0.4.0, slimctl v0.2.1, helm-slim-control-plane v0.1.3 |
| **slimctl** | v0.2.1 | control-plane v0.1.0, slim v0.4.0 |

#### Helm Chart Compatibility

| Helm Chart | Version | Deploys Component | Minimum Requirements |
|------------|---------|-------------------|---------------------|
| **helm-slim** | v0.1.9 | slim v0.4.0 | Kubernetes 1.20+ |
| **helm-slim-control-plane** | v0.1.3 | control-plane v0.1.0 | Kubernetes 1.20+ |

#### Development & Testing Tools

| Tool | Version | Works With | Purpose |
|------|---------|------------|---------|
| **slim-testutils** | v0.2.1 | All components | Testing utilities |
| **slim-bindings-examples** | v0.1.0 | slim-bindings v0.4.0 | Python examples |

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
