# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.4](https://github.com/agntcy/slim/compare/slim-mls-v0.1.3...slim-mls-v0.1.4) - 2025-11-17

### Added

- add async initialize func in the provider/verifier traits ([#917](https://github.com/agntcy/slim/pull/917))
- Integrate SPIRE-based mTLS & identity, unify TLS sources, enhance gRPC config, and add flexible metadata support ([#892](https://github.com/agntcy/slim/pull/892))
- *(mls)* identity claims integration, strengthened validation, and PoP enforcement ([#885](https://github.com/agntcy/slim/pull/885))
- async mls ([#877](https://github.com/agntcy/slim/pull/877))
- expand SharedSecret Auth from simple secret:id to HMAC tokens ([#858](https://github.com/agntcy/slim/pull/858))

### Fixed

- handle verifier.try_verify() block call properly ([#865](https://github.com/agntcy/slim/pull/865))

### Other

- implement all control message payload in protobuf ([#862](https://github.com/agntcy/slim/pull/862))

## [0.1.3](https://github.com/agntcy/slim/compare/slim-mls-v0.1.2...slim-mls-v0.1.3) - 2025-10-17

### Other

- updated the following local packages: agntcy-slim-auth, agntcy-slim-datapath

## [0.1.2](https://github.com/agntcy/slim/compare/slim-mls-v0.1.1...slim-mls-v0.1.2) - 2025-10-09

### Other

- updated the following local packages: agntcy-slim-auth, agntcy-slim-datapath

## [0.1.1](https://github.com/agntcy/slim/compare/slim-mls-v0.1.0...slim-mls-v0.1.1) - 2025-09-17

### Added

- make MLS identity provider backend agnostic ([#552](https://github.com/agntcy/slim/pull/552))

### Other

- *(agntcy-slim-mls)* release v0.1.0 ([#493](https://github.com/agntcy/slim/pull/493))

## [0.1.0](https://github.com/agntcy/slim/releases/tag/slim-mls-v0.1.0) - 2025-07-31

### Added

- add identity and mls options to python bindings ([#436](https://github.com/agntcy/slim/pull/436))
- implement key rotation proposal message exchange ([#434](https://github.com/agntcy/slim/pull/434))
- implement MLS key rotation ([#412](https://github.com/agntcy/slim/pull/412))
- integrate MLS with auth ([#385](https://github.com/agntcy/slim/pull/385))
- add mls message types in slim messages ([#386](https://github.com/agntcy/slim/pull/386))
- push and verify identities in message headers ([#384](https://github.com/agntcy/slim/pull/384))
- add the ability to drop messages from the interceptor ([#371](https://github.com/agntcy/slim/pull/371))
- implement MLS ([#307](https://github.com/agntcy/slim/pull/307))

### Other

- remove Agent and AgentType and adopt Name as application identifier ([#477](https://github.com/agntcy/slim/pull/477))
- add test application for dynamic MLS groups ([#435](https://github.com/agntcy/slim/pull/435))
- 397 remove endpoints in mls groups ([#413](https://github.com/agntcy/slim/pull/413))
- *(session)* use parking_lot to sync access to MlsState ([#401](https://github.com/agntcy/slim/pull/401))
