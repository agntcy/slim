# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.7](https://github.com/agntcy/slim/compare/slim-controller-v0.4.6...slim-controller-v0.4.7) - 2026-02-12

### Other

- updated the following local packages: agntcy-slim-config, agntcy-slim-tracing, agntcy-slim-datapath, agntcy-slim-session

## [0.4.6](https://github.com/agntcy/slim/compare/slim-controller-v0.4.5...slim-controller-v0.4.6) - 2026-02-06

### Other

- updated the following local packages: agntcy-slim-config, agntcy-slim-datapath, agntcy-slim-session, agntcy-slim-tracing

## [0.4.5](https://github.com/agntcy/slim/compare/slim-controller-v0.4.4...slim-controller-v0.4.5) - 2026-01-30

### Other

- updated the following local packages: agntcy-slim-config, agntcy-slim-tracing, agntcy-slim-datapath, agntcy-slim-session

## [0.4.4](https://github.com/agntcy/slim/compare/slim-controller-v0.4.3...slim-controller-v0.4.4) - 2026-01-29

### Added

- Support different trust domains in auto route setup ([#1001](https://github.com/agntcy/slim/pull/1001))
- *(bindings)* expose identity configuration ([#1092](https://github.com/agntcy/slim/pull/1092))
- send group acknowledge from the session ([#1050](https://github.com/agntcy/slim/pull/1050))
- make backoff retry configurable ([#991](https://github.com/agntcy/slim/pull/991))

### Fixed

- *(controller)* start the controller service only if the related config is provided ([#1054](https://github.com/agntcy/slim/pull/1054))
- *(bindings)* improve identity error handling ([#1042](https://github.com/agntcy/slim/pull/1042))

### Other

- *(bindings)* do not expose tokio-specific APIs to foreign async calls ([#1110](https://github.com/agntcy/slim/pull/1110))
- *(lint)* use latest version of tools ([#1067](https://github.com/agntcy/slim/pull/1067))
- unified typed error handling across core crates ([#976](https://github.com/agntcy/slim/pull/976))

## [0.4.3](https://github.com/agntcy/slim/compare/slim-controller-v0.4.2...slim-controller-v0.4.3) - 2025-11-21

### Fixed

- flaky integration test ([#981](https://github.com/agntcy/slim/pull/981))
- add the subcription messages queue in controller ([#930](https://github.com/agntcy/slim/pull/930))

## [0.4.2](https://github.com/agntcy/slim/compare/slim-controller-v0.4.1...slim-controller-v0.4.2) - 2025-11-17

### Added

- add backoff retry ([#939](https://github.com/agntcy/slim/pull/939))
- Integrate SPIRE-based mTLS & identity, unify TLS sources, enhance gRPC config, and add flexible metadata support ([#892](https://github.com/agntcy/slim/pull/892))

### Fixed

- add original MsgID to all response messages ([#891](https://github.com/agntcy/slim/pull/891))
- Handle route connection faliures and node connection detail changes ([#833](https://github.com/agntcy/slim/pull/833))

### Other

- unify multicast and P2P session handling ([#904](https://github.com/agntcy/slim/pull/904))
- implement all control message payload in protobuf ([#862](https://github.com/agntcy/slim/pull/862))
- *(data-plane)* update project dependencies ([#861](https://github.com/agntcy/slim/pull/861))

## [0.4.1](https://github.com/agntcy/slim/compare/slim-controller-v0.4.0...slim-controller-v0.4.1) - 2025-10-17

### Other

- updated the following local packages: agntcy-slim-auth, agntcy-slim-config, agntcy-slim-datapath, agntcy-slim-tracing

## [0.4.0](https://github.com/agntcy/slim/compare/slim-controller-v0.3.0...slim-controller-v0.4.0) - 2025-10-09

### Added

- implement control plane group management ([#554](https://github.com/agntcy/slim/pull/554))
- handle updates from SLIM nodes ([#708](https://github.com/agntcy/slim/pull/708))

### Other

- upgrade to rust toolchain 1.90.0 ([#730](https://github.com/agntcy/slim/pull/730))
- rename sessions in python bindings ([#698](https://github.com/agntcy/slim/pull/698))

## [0.3.0](https://github.com/agntcy/slim/compare/slim-controller-v0.2.0...slim-controller-v0.3.0) - 2025-09-17

### Added

- notify controller with new subscriptions ([#611](https://github.com/agntcy/slim/pull/611))
- replace pubsub with dataplane in the node-config ([#591](https://github.com/agntcy/slim/pull/591))
- Update buf ci config ([#532](https://github.com/agntcy/slim/pull/532))
- Update SB API in control-plane to support group crud operations ([#478](https://github.com/agntcy/slim/pull/478))

### Other

- SLIM node ID should be unique in a deployment ([#630](https://github.com/agntcy/slim/pull/630))

## [0.2.0](https://github.com/agntcy/slim/compare/slim-controller-v0.1.1...slim-controller-v0.2.0) - 2025-07-31

### Added

- control plane service & slimctl cp commands ([#388](https://github.com/agntcy/slim/pull/388))
- add client connections to control plane ([#429](https://github.com/agntcy/slim/pull/429))
- add node register call to proto ([#406](https://github.com/agntcy/slim/pull/406))
- *(proto)* introduce SessionType in message header ([#410](https://github.com/agntcy/slim/pull/410))
- *(control-plane)* handle all configuration parameters when creating a new connection ([#360](https://github.com/agntcy/slim/pull/360))

### Other

- remove Agent and AgentType and adopt Name as application identifier ([#477](https://github.com/agntcy/slim/pull/477))

## [0.1.1](https://github.com/agntcy/slim/compare/slim-controller-v0.1.0...slim-controller-v0.1.1) - 2025-05-14

### Other

- *(slim-controller)* release v0.1.0 ([#250](https://github.com/agntcy/slim/pull/250))

## [0.1.0](https://github.com/agntcy/slim/releases/tag/slim-controller-v0.1.0) - 2025-05-14

### Added

- implement control API ([#147](https://github.com/agntcy/slim/pull/147))

### Fixed

- *(datapath)* keep protobuf in crate ([#248](https://github.com/agntcy/slim/pull/248))

### Other

- add integration test suite ([#233](https://github.com/agntcy/slim/pull/233))
