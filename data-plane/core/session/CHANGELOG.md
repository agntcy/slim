# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.7](https://github.com/agntcy/slim/compare/slim-session-v0.1.6...slim-session-v0.1.7) - 2026-02-27

### Other

- updated the following local packages: agntcy-slim-auth, agntcy-slim-datapath, agntcy-slim-mls

## [0.1.6](https://github.com/agntcy/slim/compare/slim-session-v0.1.5...slim-session-v0.1.6) - 2026-02-12

### Added

- slimrpc-compiler for golang + example ([#1163](https://github.com/agntcy/slim/pull/1163))

## [0.1.5](https://github.com/agntcy/slim/compare/slim-session-v0.1.4...slim-session-v0.1.5) - 2026-02-06

### Added

- remove lock from mls state ([#1203](https://github.com/agntcy/slim/pull/1203))

### Other

- *(data-plane)* upgrade to rust 1.93 ([#1190](https://github.com/agntcy/slim/pull/1190))

## [0.1.4](https://github.com/agntcy/slim/compare/slim-session-v0.1.3...slim-session-v0.1.4) - 2026-01-30

### Other

- updated the following local packages: agntcy-slim-datapath, agntcy-slim-mls

## [0.1.3](https://github.com/agntcy/slim/compare/slim-session-v0.1.2...slim-session-v0.1.3) - 2026-01-29

### Added

- add reference count to subscription table ([#1143](https://github.com/agntcy/slim/pull/1143))
- *(session)* Add direction to slim app to control message flow ([#1121](https://github.com/agntcy/slim/pull/1121))
- generate python bindings with uniffi ([#1046](https://github.com/agntcy/slim/pull/1046))
- *(bindings)* expose participant list to the application ([#1089](https://github.com/agntcy/slim/pull/1089))
- send group acknowledge from the session ([#1050](https://github.com/agntcy/slim/pull/1050))
- *(bindings)* expose complete configuration for auth and creating clients, servers ([#1084](https://github.com/agntcy/slim/pull/1084))
- *(session)* handle moderator unexpected stop ([#1024](https://github.com/agntcy/slim/pull/1024))
- Update group state on unexpected application stop ([#1014](https://github.com/agntcy/slim/pull/1014))
- detect and handle unexpected participant disconnections ([#1004](https://github.com/agntcy/slim/pull/1004))

### Fixed

- add missing routes to participants ([#1131](https://github.com/agntcy/slim/pull/1131))
- check if a participant is already in the group before invite ([#1085](https://github.com/agntcy/slim/pull/1085))
- *(session)* send ping messages to the right destination ([#1066](https://github.com/agntcy/slim/pull/1066))
- *(session)* route dataplane errors to correct session ([#1056](https://github.com/agntcy/slim/pull/1056))
- *(session)* remove participants from the group list ([#1059](https://github.com/agntcy/slim/pull/1059))
- *(bindings)* improve identity error handling ([#1042](https://github.com/agntcy/slim/pull/1042))
- *(session)* correctly remove routes on session close ([#1039](https://github.com/agntcy/slim/pull/1039))
- *(moderator_task.rs)* typo ([#1008](https://github.com/agntcy/slim/pull/1008))

### Other

- unified typed error handling across core crates ([#976](https://github.com/agntcy/slim/pull/976))

## [0.1.2](https://github.com/agntcy/slim/compare/slim-session-v0.1.1...slim-session-v0.1.2) - 2025-11-21

### Added

- add publish_to on group session ([#975](https://github.com/agntcy/slim/pull/975))

### Fixed

- remove participants and ack management in controller sender ([#987](https://github.com/agntcy/slim/pull/987))

## [0.1.1](https://github.com/agntcy/slim/compare/slim-session-v0.1.0...slim-session-v0.1.1) - 2025-11-17

### Added

- *(session)* graceful session draining + reliable blocking API completion ([#924](https://github.com/agntcy/slim/pull/924))
- improve logging configuration ([#943](https://github.com/agntcy/slim/pull/943))
- add async initialize func in the provider/verifier traits ([#917](https://github.com/agntcy/slim/pull/917))
- Integrate SPIRE-based mTLS & identity, unify TLS sources, enhance gRPC config, and add flexible metadata support ([#892](https://github.com/agntcy/slim/pull/892))
- *(mls)* identity claims integration, strengthened validation, and PoP enforcement ([#885](https://github.com/agntcy/slim/pull/885))
- async mls ([#877](https://github.com/agntcy/slim/pull/877))
- expand SharedSecret Auth from simple secret:id to HMAC tokens ([#858](https://github.com/agntcy/slim/pull/858))
- derive name ID part from identity token ([#851](https://github.com/agntcy/slim/pull/851))x
- *(session)* create sender and receiver ([#836](https://github.com/agntcy/slim/pull/836))

### Fixed

- *(session)* prevent session queue saturation ([#903](https://github.com/agntcy/slim/pull/903))

### Other

- unify multicast and P2P session handling ([#904](https://github.com/agntcy/slim/pull/904))
- split mls state and channel endpoint ([#875](https://github.com/agntcy/slim/pull/875))
- implement all control message payload in protobuf ([#862](https://github.com/agntcy/slim/pull/862))
- *(agntcy-slim-session)* release v0.1.0 ([#856](https://github.com/agntcy/slim/pull/856))

## [0.1.0](https://github.com/agntcy/slim/releases/tag/slim-session-v0.1.0) - 2025-10-17

### Added

- move session code in a new crate ([#828](https://github.com/agntcy/slim/pull/828))

### Fixed

- *(session)* correctly handle multiple subscriptions ([#838](https://github.com/agntcy/slim/pull/838))
