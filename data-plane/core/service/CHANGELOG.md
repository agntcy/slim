# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.8.1](https://github.com/agntcy/slim/compare/slim-service-v0.8.0...slim-service-v0.8.1) - 2025-11-17

### Added

- enable spire as token provider for clients ([#945](https://github.com/agntcy/slim/pull/945))
- *(session)* graceful session draining + reliable blocking API completion ([#924](https://github.com/agntcy/slim/pull/924))
- Integrate SPIRE-based mTLS & identity, unify TLS sources, enhance gRPC config, and add flexible metadata support ([#892](https://github.com/agntcy/slim/pull/892))
- expand SharedSecret Auth from simple secret:id to HMAC tokens ([#858](https://github.com/agntcy/slim/pull/858))
- derive name ID part from identity token ([#851](https://github.com/agntcy/slim/pull/851))x

### Fixed

- *(session)* prevent session queue saturation ([#903](https://github.com/agntcy/slim/pull/903))
- *(service)* disconnect API ([#890](https://github.com/agntcy/slim/pull/890))
- *(app.rs)* get app name from local property ([#859](https://github.com/agntcy/slim/pull/859))

### Other

- unify multicast and P2P session handling ([#904](https://github.com/agntcy/slim/pull/904))
- implement all control message payload in protobuf ([#862](https://github.com/agntcy/slim/pull/862))
- common rust infrastructure for language bindings ([#840](https://github.com/agntcy/slim/pull/840))

## [0.8.0](https://github.com/agntcy/slim/compare/slim-service-v0.7.0...slim-service-v0.8.0) - 2025-10-17

### Added

- move session code in a new crate ([#828](https://github.com/agntcy/slim/pull/828))

### Fixed

- *(session)* correctly handle multiple subscriptions ([#838](https://github.com/agntcy/slim/pull/838))

## [0.7.0](https://github.com/agntcy/slim/compare/slim-service-v0.6.0...slim-service-v0.7.0) - 2025-10-09

### Added

- *(python/examples)* allow each participant to publish ([#778](https://github.com/agntcy/slim/pull/778))
- implement control plane group management ([#554](https://github.com/agntcy/slim/pull/554))
- remove bearer auth in favour of static jwt ([#774](https://github.com/agntcy/slim/pull/774))
- *(python/bindings)* improve publish function ([#749](https://github.com/agntcy/slim/pull/749))
- *(session)* introduce session metadata ([#744](https://github.com/agntcy/slim/pull/744))
- *(pysession)* expose session type, src and dst names ([#737](https://github.com/agntcy/slim/pull/737))
- *(multicast)* remove moderator parameter from configuration ([#739](https://github.com/agntcy/slim/pull/739))
- improve point to point session with sender/receiver buffer ([#735](https://github.com/agntcy/slim/pull/735))
- [**breaking**] refactor session receive() API ([#731](https://github.com/agntcy/slim/pull/731))
- handle updates from SLIM nodes ([#708](https://github.com/agntcy/slim/pull/708))
- add string name on pub messages ([#693](https://github.com/agntcy/slim/pull/693))

### Fixed

- avoid panic sending errors to the local application ([#814](https://github.com/agntcy/slim/pull/814))
- *(python-bindings)* remove destination_name property ([#751](https://github.com/agntcy/slim/pull/751))

### Other

- *(python/bindings)* remove anycast session and rename unicast and multicast ([#795](https://github.com/agntcy/slim/pull/795))
- upgrade to rust toolchain 1.90.0 ([#730](https://github.com/agntcy/slim/pull/730))
- rename sessions in python bindings ([#698](https://github.com/agntcy/slim/pull/698))
- *(service)* session files in separate module ([#695](https://github.com/agntcy/slim/pull/695))
- rename session types in rust code ([#679](https://github.com/agntcy/slim/pull/679))

## [0.6.0](https://github.com/agntcy/slim/compare/slim-service-v0.5.0...slim-service-v0.6.0) - 2025-09-17

### Added

- notify controller with new subscriptions ([#611](https://github.com/agntcy/slim/pull/611))
- Introduce SRPC + Native A2A integration  ([#550](https://github.com/agntcy/slim/pull/550))
- replace pubsub with dataplane in the node-config ([#591](https://github.com/agntcy/slim/pull/591))
- make MLS identity provider backend agnostic ([#552](https://github.com/agntcy/slim/pull/552))

### Fixed

- fix ff session ([#538](https://github.com/agntcy/slim/pull/538))

### Other

- SLIM node ID should be unique in a deployment ([#630](https://github.com/agntcy/slim/pull/630))

## [0.5.0](https://github.com/agntcy/slim/compare/slim-service-v0.4.2...slim-service-v0.5.0) - 2025-07-31

### Added

- get source and destination name form python ([#485](https://github.com/agntcy/slim/pull/485))
- *(python-bindings)* update examples and make them packageable ([#468](https://github.com/agntcy/slim/pull/468))
- process concurrent modification to the group ([#451](https://github.com/agntcy/slim/pull/451))
- add identity and mls options to python bindings ([#436](https://github.com/agntcy/slim/pull/436))
- improve handling of commit broadcast ([#433](https://github.com/agntcy/slim/pull/433))
- implement key rotation proposal message exchange ([#434](https://github.com/agntcy/slim/pull/434))
- add client connections to control plane ([#429](https://github.com/agntcy/slim/pull/429))
- implement MLS key rotation ([#412](https://github.com/agntcy/slim/pull/412))
- *(proto)* introduce SessionType in message header ([#410](https://github.com/agntcy/slim/pull/410))
- *(channel_endpoint)* add error handling ([#409](https://github.com/agntcy/slim/pull/409))
- do no create session on discovery request ([#402](https://github.com/agntcy/slim/pull/402))
- integrate MLS with auth ([#385](https://github.com/agntcy/slim/pull/385))
- add mls message types in slim messages ([#386](https://github.com/agntcy/slim/pull/386))
- push and verify identities in message headers ([#384](https://github.com/agntcy/slim/pull/384))
- add auth support in sessions ([#382](https://github.com/agntcy/slim/pull/382))
- channel creation in session layer ([#374](https://github.com/agntcy/slim/pull/374))
- add the ability to drop messages from the interceptor ([#371](https://github.com/agntcy/slim/pull/371))
- implement MLS ([#307](https://github.com/agntcy/slim/pull/307))
- add identity into the SLIM message ([#342](https://github.com/agntcy/slim/pull/342))
- *(data-plane)* upgrade to rust 1.87 ([#317](https://github.com/agntcy/slim/pull/317))

### Fixed

- prevent message publication before mls setup ([#458](https://github.com/agntcy/slim/pull/458))
- remove all state on session close ([#449](https://github.com/agntcy/slim/pull/449))
- mls update timer duration ([#437](https://github.com/agntcy/slim/pull/437))
- *(channel_endpoint)* extend mls for all sessions ([#411](https://github.com/agntcy/slim/pull/411))
- fix building problem ([#422](https://github.com/agntcy/slim/pull/422))
- [**breaking**] remove request-reply session type ([#416](https://github.com/agntcy/slim/pull/416))
- *(auth)* make simple identity usable for groups ([#387](https://github.com/agntcy/slim/pull/387))

### Other

- remove Agent and AgentType and adopt Name as application identifier ([#477](https://github.com/agntcy/slim/pull/477))
- 397 remove endpoints in mls groups ([#413](https://github.com/agntcy/slim/pull/413))
- *(session)* use parking_lot to sync access to MlsState ([#401](https://github.com/agntcy/slim/pull/401))
- fix test channel endpoint ([#405](https://github.com/agntcy/slim/pull/405))
- *(streaming)* remove lock from channel_endpoint ([#399](https://github.com/agntcy/slim/pull/399))

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
