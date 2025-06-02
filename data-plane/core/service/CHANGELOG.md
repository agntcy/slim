# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.2](https://github.com/agntcy/slim/compare/slim-service-v0.4.1...slim-service-v0.4.2) - 2025-05-14

### Other

- updated the following local packages: slim-controller

## [0.4.1](https://github.com/agntcy/slim/compare/slim-service-v0.4.0...slim-service-v0.4.1) - 2025-05-14

### Added

- improve tracing in slim ([#237](https://github.com/agntcy/slim/pull/237))
- implement control API ([#147](https://github.com/agntcy/slim/pull/147))

### Fixed

- shut down controller server properly ([#202](https://github.com/agntcy/slim/pull/202))
- *(python-bindings)* test failure ([#194](https://github.com/agntcy/slim/pull/194))

## [0.4.0](https://github.com/agntcy/slim/compare/slim-service-v0.3.0...slim-service-v0.4.0) - 2025-04-24

### Added

- *(session)* add default config for sessions created upon message reception ([#181](https://github.com/agntcy/slim/pull/181))
- *(session)* add tests for session deletion ([#179](https://github.com/agntcy/slim/pull/179))
- add beacon messages from the producer for streaming and pub/sub ([#177](https://github.com/agntcy/slim/pull/177))
- *(python-bindings)* add session deletion API ([#176](https://github.com/agntcy/slim/pull/176))
- *(python-bindings)* improve configuration handling and further refactoring ([#167](https://github.com/agntcy/slim/pull/167))
- *(data-plane)* support for multiple servers ([#173](https://github.com/agntcy/slim/pull/173))
- add exponential timers ([#172](https://github.com/agntcy/slim/pull/172))
- *(session layer)* send rtx error if the packet is not in the producer buffer ([#166](https://github.com/agntcy/slim/pull/166))

### Fixed

- *(data-plane)* make new linter version happy ([#184](https://github.com/agntcy/slim/pull/184))

### Other

- declare all dependencies in workspace Cargo.toml ([#187](https://github.com/agntcy/slim/pull/187))
- *(data-plane)* tonic 0.12.3 -> 0.13 ([#170](https://github.com/agntcy/slim/pull/170))
- upgrade to rust edition 2024 and toolchain 1.86.0 ([#164](https://github.com/agntcy/slim/pull/164))

## [0.3.0](https://github.com/agntcy/slim/compare/slim-service-v0.2.1...slim-service-v0.3.0) - 2025-04-08

### Added

- *(python-bindings)* add examples ([#153](https://github.com/agntcy/slim/pull/153))
- add pub/sub session layer ([#146](https://github.com/agntcy/slim/pull/146))
- streaming test app ([#144](https://github.com/agntcy/slim/pull/144))
- streaming session type ([#132](https://github.com/agntcy/slim/pull/132))
- request/reply session type ([#124](https://github.com/agntcy/slim/pull/124))
- add timers for rtx ([#117](https://github.com/agntcy/slim/pull/117))
- rename protobuf fields ([#116](https://github.com/agntcy/slim/pull/116))
- add receiver buffer ([#107](https://github.com/agntcy/slim/pull/107))
- producer buffer ([#105](https://github.com/agntcy/slim/pull/105))
- *(data-plane/service)* [**breaking**] first draft of session layer ([#106](https://github.com/agntcy/slim/pull/106))

### Other

- *(python-bindings)* streaming and pubsub sessions ([#152](https://github.com/agntcy/slim/pull/152))
- *(session)* make agent source part of session commons ([#151](https://github.com/agntcy/slim/pull/151))
- *(python-bindings)* add request/reply tests ([#142](https://github.com/agntcy/slim/pull/142))
- remove locks in streaming session layer ([#145](https://github.com/agntcy/slim/pull/145))
- improve utils classes and simplify message processor ([#131](https://github.com/agntcy/slim/pull/131))
- *(service)* simplify session trait with async_trait ([#121](https://github.com/agntcy/slim/pull/121))
- add Python SDK test cases for failure scenarios
- update copyright ([#109](https://github.com/agntcy/slim/pull/109))

## [0.2.1](https://github.com/agntcy/slim/compare/slim-service-v0.2.0...slim-service-v0.2.1) - 2025-03-19

### Other

- updated the following local packages: slim-datapath

## [0.2.0](https://github.com/agntcy/slim/compare/slim-service-v0.1.8...slim-service-v0.2.0) - 2025-03-19

### Other

- use same API for send_to and publish ([#89](https://github.com/agntcy/slim/pull/89))

## [0.1.8](https://github.com/agntcy/slim/compare/slim-service-v0.1.7...slim-service-v0.1.8) - 2025-03-18

### Added

- new message format ([#88](https://github.com/agntcy/slim/pull/88))

## [0.1.7](https://github.com/agntcy/slim/compare/slim-service-v0.1.6...slim-service-v0.1.7) - 2025-03-18

### Added

- propagate context to enable distributed tracing ([#90](https://github.com/agntcy/slim/pull/90))

## [0.1.6](https://github.com/agntcy/slim/compare/slim-service-v0.1.5...slim-service-v0.1.6) - 2025-03-12

### Added

- notify local app if a message is not processed correctly ([#72](https://github.com/agntcy/slim/pull/72))

## [0.1.5](https://github.com/agntcy/slim/compare/slim-service-v0.1.4...slim-service-v0.1.5) - 2025-03-11

### Other

- *(slim-config)* release v0.1.4 ([#79](https://github.com/agntcy/slim/pull/79))

## [0.1.4](https://github.com/agntcy/slim/compare/slim-service-v0.1.3...slim-service-v0.1.4) - 2025-02-28

### Added

- handle disconnection events (#67)

## [0.1.3](https://github.com/agntcy/slim/compare/slim-service-v0.1.2...slim-service-v0.1.3) - 2025-02-28

### Added

- add message handling metrics

## [0.1.2](https://github.com/agntcy/slim/compare/slim-service-v0.1.1...slim-service-v0.1.2) - 2025-02-19

### Other

- updated the following local packages: slim-datapath

## [0.1.1](https://github.com/agntcy/slim/compare/slim-service-v0.1.0...slim-service-v0.1.1) - 2025-02-14

### Added

- implement opentelemetry tracing subscriber

## [0.1.0](https://github.com/agntcy/slim/releases/tag/slim-service-v0.1.0) - 2025-02-10

### Added

- Stage the first commit of SLIM (#3)

### Other

- reduce the number of crates to publish (#10)
