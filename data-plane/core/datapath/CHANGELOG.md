# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.10.2](https://github.com/agntcy/slim/compare/slim-datapath-v0.10.1...slim-datapath-v0.10.2) - 2025-11-17

### Added

- *(session)* graceful session draining + reliable blocking API completion ([#924](https://github.com/agntcy/slim/pull/924))
- connection drop controller update ([#901](https://github.com/agntcy/slim/pull/901))
- Integrate SPIRE-based mTLS & identity, unify TLS sources, enhance gRPC config, and add flexible metadata support ([#892](https://github.com/agntcy/slim/pull/892))
- *(auth)* add support for setting custom claims while getting the token ([#879](https://github.com/agntcy/slim/pull/879))
- expand SharedSecret Auth from simple secret:id to HMAC tokens ([#858](https://github.com/agntcy/slim/pull/858))

### Fixed

- *(service)* disconnect API ([#890](https://github.com/agntcy/slim/pull/890))

### Other

- unify multicast and P2P session handling ([#904](https://github.com/agntcy/slim/pull/904))
- implement all control message payload in protobuf ([#862](https://github.com/agntcy/slim/pull/862))
- *(data-plane)* update project dependencies ([#861](https://github.com/agntcy/slim/pull/861))

## [0.10.1](https://github.com/agntcy/slim/compare/slim-datapath-v0.10.0...slim-datapath-v0.10.1) - 2025-10-17

### Fixed

- *(session)* correctly handle multiple subscriptions ([#838](https://github.com/agntcy/slim/pull/838))

## [0.10.0](https://github.com/agntcy/slim/compare/slim-datapath-v0.9.0...slim-datapath-v0.10.0) - 2025-10-09

### Added

- implement control plane group management ([#554](https://github.com/agntcy/slim/pull/554))
- [**breaking**] refactor session receive() API ([#731](https://github.com/agntcy/slim/pull/731))
- add string name on pub messages ([#693](https://github.com/agntcy/slim/pull/693))

### Fixed

- *(subscription-table)* wrong iterator when matching over multiple output connection ([#815](https://github.com/agntcy/slim/pull/815))

### Other

- upgrade to rust toolchain 1.90.0 ([#730](https://github.com/agntcy/slim/pull/730))
- rename sessions in python bindings ([#698](https://github.com/agntcy/slim/pull/698))
- rename session types in rust code ([#679](https://github.com/agntcy/slim/pull/679))

## [0.9.0](https://github.com/agntcy/slim/compare/slim-datapath-v0.8.0...slim-datapath-v0.9.0) - 2025-09-17

### Added

- notify controller with new subscriptions ([#611](https://github.com/agntcy/slim/pull/611))

### Fixed

- fix ff session ([#538](https://github.com/agntcy/slim/pull/538))

## [0.8.0](https://github.com/agntcy/slim/compare/slim-datapath-v0.7.0...slim-datapath-v0.8.0) - 2025-07-31

### Added

- implement key rotation proposal message exchange ([#434](https://github.com/agntcy/slim/pull/434))
- *(proto)* introduce SessionType in message header ([#410](https://github.com/agntcy/slim/pull/410))
- *(control-plane)* handle all configuration parameters when creating a new connection ([#360](https://github.com/agntcy/slim/pull/360))
- add mls message types in slim messages ([#386](https://github.com/agntcy/slim/pull/386))
- push and verify identities in message headers ([#384](https://github.com/agntcy/slim/pull/384))
- add auth support in sessions ([#382](https://github.com/agntcy/slim/pull/382))
- channel creation in session layer ([#374](https://github.com/agntcy/slim/pull/374))
- implement MLS ([#307](https://github.com/agntcy/slim/pull/307))
- derive name id from provided identity ([#345](https://github.com/agntcy/slim/pull/345))
- add identity into the SLIM message ([#342](https://github.com/agntcy/slim/pull/342))
- *(data-plane)* upgrade to rust 1.87 ([#317](https://github.com/agntcy/slim/pull/317))

### Fixed

- *(channel_endpoint)* extend mls for all sessions ([#411](https://github.com/agntcy/slim/pull/411))
- [**breaking**] remove request-reply session type ([#416](https://github.com/agntcy/slim/pull/416))

### Other

- remove Agent and AgentType and adopt Name as application identifier ([#477](https://github.com/agntcy/slim/pull/477))

## [0.7.0](https://github.com/agntcy/slim/compare/slim-datapath-v0.6.0...slim-datapath-v0.7.0) - 2025-05-14

### Added

- *(subscription_table)* add foreach to iterate over the table ([#240](https://github.com/agntcy/slim/pull/240))
- *(pool.rs)* add iterator to pool ([#236](https://github.com/agntcy/slim/pull/236))
- improve tracing in slim ([#237](https://github.com/agntcy/slim/pull/237))
- implement control API ([#147](https://github.com/agntcy/slim/pull/147))

## [0.6.0](https://github.com/agntcy/slim/compare/slim-datapath-v0.5.0...slim-datapath-v0.6.0) - 2025-04-24

### Added

- improve configuration handling for tracing ([#186](https://github.com/agntcy/slim/pull/186))
- add beacon messages from the producer for streaming and pub/sub ([#177](https://github.com/agntcy/slim/pull/177))
- *(python-bindings)* improve configuration handling and further refactoring ([#167](https://github.com/agntcy/slim/pull/167))
- *(session layer)* send rtx error if the packet is not in the producer buffer ([#166](https://github.com/agntcy/slim/pull/166))

### Fixed

- *(data-plane)* make new linter version happy ([#184](https://github.com/agntcy/slim/pull/184))

### Other

- declare all dependencies in workspace Cargo.toml ([#187](https://github.com/agntcy/slim/pull/187))
- *(data-plane)* tonic 0.12.3 -> 0.13 ([#170](https://github.com/agntcy/slim/pull/170))
- upgrade to rust edition 2024 and toolchain 1.86.0 ([#164](https://github.com/agntcy/slim/pull/164))

## [0.5.0](https://github.com/agntcy/slim/compare/slim-datapath-v0.4.2...slim-datapath-v0.5.0) - 2025-04-08

### Added

- *(python-bindings)* add examples ([#153](https://github.com/agntcy/slim/pull/153))
- add pub/sub session layer ([#146](https://github.com/agntcy/slim/pull/146))
- streaming session type ([#132](https://github.com/agntcy/slim/pull/132))
- request/reply session type ([#124](https://github.com/agntcy/slim/pull/124))
- add timers for rtx ([#117](https://github.com/agntcy/slim/pull/117))
- rename protobuf fields ([#116](https://github.com/agntcy/slim/pull/116))
- add receiver buffer ([#107](https://github.com/agntcy/slim/pull/107))
- producer buffer ([#105](https://github.com/agntcy/slim/pull/105))
- *(data-plane/service)* [**breaking**] first draft of session layer ([#106](https://github.com/agntcy/slim/pull/106))

### Fixed

- *(python-bindings)* fix python examples ([#120](https://github.com/agntcy/slim/pull/120))
- *(datapath)* fix reconnection logic ([#119](https://github.com/agntcy/slim/pull/119))

### Other

- *(python-bindings)* streaming and pubsub sessions ([#152](https://github.com/agntcy/slim/pull/152))
- *(python-bindings)* add request/reply tests ([#142](https://github.com/agntcy/slim/pull/142))
- improve utils classes and simplify message processor ([#131](https://github.com/agntcy/slim/pull/131))
- improve connection pool performance ([#125](https://github.com/agntcy/slim/pull/125))
- update copyright ([#109](https://github.com/agntcy/slim/pull/109))

## [0.4.2](https://github.com/agntcy/slim/compare/slim-datapath-v0.4.1...slim-datapath-v0.4.2) - 2025-03-19

### Added

- improve message processing file ([#101](https://github.com/agntcy/slim/pull/101))

## [0.4.1](https://github.com/agntcy/slim/compare/slim-datapath-v0.4.0...slim-datapath-v0.4.1) - 2025-03-19

### Added

- *(tables)* do not require Default/Clone traits for elements stored in pool ([#97](https://github.com/agntcy/slim/pull/97))

### Other

- use same API for send_to and publish ([#89](https://github.com/agntcy/slim/pull/89))

## [0.4.0](https://github.com/agntcy/slim/compare/slim-datapath-v0.3.1...slim-datapath-v0.4.0) - 2025-03-18

### Added

- new message format ([#88](https://github.com/agntcy/slim/pull/88))

## [0.3.1](https://github.com/agntcy/slim/compare/slim-datapath-v0.3.0...slim-datapath-v0.3.1) - 2025-03-18

### Added

- propagate context to enable distributed tracing ([#90](https://github.com/agntcy/slim/pull/90))

## [0.3.0](https://github.com/agntcy/slim/compare/slim-datapath-v0.2.1...slim-datapath-v0.3.0) - 2025-03-12

### Added

- notify local app if a message is not processed correctly ([#72](https://github.com/agntcy/slim/pull/72))

## [0.2.1](https://github.com/agntcy/slim/compare/slim-datapath-v0.2.0...slim-datapath-v0.2.1) - 2025-03-11

### Other

- *(slim-config)* release v0.1.4 ([#79](https://github.com/agntcy/slim/pull/79))

## [0.2.0](https://github.com/agntcy/slim/compare/slim-datapath-v0.1.2...slim-datapath-v0.2.0) - 2025-02-28

### Added

- handle disconnection events (#67)

## [0.1.2](https://github.com/agntcy/slim/compare/slim-datapath-v0.1.1...slim-datapath-v0.1.2) - 2025-02-28

### Added

- add message handling metrics

## [0.1.1](https://github.com/agntcy/slim/compare/slim-datapath-v0.1.0...slim-datapath-v0.1.1) - 2025-02-19

### Added

- *(tables)* distinguish local and remote connections in the subscription table (#55)

## [0.1.0](https://github.com/agntcy/slim/releases/tag/slim-data-path-v0.1.0) - 2025-02-09

### Added

- Stage the first commit of SLIM (#3)

### Other

- release process for rust crates
