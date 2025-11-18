# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.1](https://github.com/agntcy/slim/compare/slim-auth-v0.4.0...slim-auth-v0.4.1) - 2025-11-17

### Added

- enable spire as token provider for clients ([#945](https://github.com/agntcy/slim/pull/945))
- *(session)* graceful session draining + reliable blocking API completion ([#924](https://github.com/agntcy/slim/pull/924))
- add async initialize func in the provider/verifier traits ([#917](https://github.com/agntcy/slim/pull/917))
- Integrate SPIRE-based mTLS & identity, unify TLS sources, enhance gRPC config, and add flexible metadata support ([#892](https://github.com/agntcy/slim/pull/892))
- *(mls)* identity claims integration, strengthened validation, and PoP enforcement ([#885](https://github.com/agntcy/slim/pull/885))
- *(auth)* add support for setting custom claims while getting the token ([#879](https://github.com/agntcy/slim/pull/879))
- expand SharedSecret Auth from simple secret:id to HMAC tokens ([#858](https://github.com/agntcy/slim/pull/858))
- derive name ID part from identity token ([#851](https://github.com/agntcy/slim/pull/851))x
- implementation of Spire for fetching the certificates/token directly from SPIFFE Workload API ([#646](https://github.com/agntcy/slim/pull/646))

### Fixed

- *(spire)* get all x509 bundles ([#960](https://github.com/agntcy/slim/pull/960))
- handle verifier.try_verify() block call properly ([#865](https://github.com/agntcy/slim/pull/865))

### Other

- unify multicast and P2P session handling ([#904](https://github.com/agntcy/slim/pull/904))
- *(data-plane)* update project dependencies ([#861](https://github.com/agntcy/slim/pull/861))

## [0.4.0](https://github.com/agntcy/slim/compare/slim-auth-v0.3.1...slim-auth-v0.4.0) - 2025-10-17

### Added

- implementation of Identity provider client credential flow ([#464](https://github.com/agntcy/slim/pull/464))

## [0.3.1](https://github.com/agntcy/slim/compare/slim-auth-v0.3.0...slim-auth-v0.3.1) - 2025-10-09

### Added

- implement control plane group management ([#554](https://github.com/agntcy/slim/pull/554))
- remove bearer auth in favour of static jwt ([#774](https://github.com/agntcy/slim/pull/774))

### Other

- upgrade to rust toolchain 1.90.0 ([#730](https://github.com/agntcy/slim/pull/730))

## [0.3.0](https://github.com/agntcy/slim/compare/slim-auth-v0.2.0...slim-auth-v0.3.0) - 2025-09-17

### Added

- make MLS identity provider backend agnostic ([#552](https://github.com/agntcy/slim/pull/552))

### Fixed

- *(python-bindings)* default crypto provider initialization for Reqwest crate ([#706](https://github.com/agntcy/slim/pull/706))

## [0.2.0](https://github.com/agntcy/slim/compare/slim-auth-v0.1.0...slim-auth-v0.2.0) - 2025-07-31

### Added

- *(python-bindings)* update examples and make them packageable ([#468](https://github.com/agntcy/slim/pull/468))
- *(auth)* support JWK as decoding keys ([#461](https://github.com/agntcy/slim/pull/461))
- add identity and mls options to python bindings ([#436](https://github.com/agntcy/slim/pull/436))
- implement MLS key rotation ([#412](https://github.com/agntcy/slim/pull/412))
- *(control-plane)* handle all configuration parameters when creating a new connection ([#360](https://github.com/agntcy/slim/pull/360))
- push and verify identities in message headers ([#384](https://github.com/agntcy/slim/pull/384))
- add auth support in sessions ([#382](https://github.com/agntcy/slim/pull/382))
- implement MLS ([#307](https://github.com/agntcy/slim/pull/307))
- support hot reload of TLS certificates ([#359](https://github.com/agntcy/slim/pull/359))
- *(auth)* get JWT from file ([#358](https://github.com/agntcy/slim/pull/358))
- *(config)* update the public/private key on file change ([#356](https://github.com/agntcy/slim/pull/356))
- *(auth)* introduce token provider trait ([#357](https://github.com/agntcy/slim/pull/357))
- *(auth)* jwt middleware ([#352](https://github.com/agntcy/slim/pull/352))

### Fixed

- *(auth)* make simple identity usable for groups ([#387](https://github.com/agntcy/slim/pull/387))

### Other

- remove Agent and AgentType and adopt Name as application identifier ([#477](https://github.com/agntcy/slim/pull/477))
- *(session)* use parking_lot to sync access to MlsState ([#401](https://github.com/agntcy/slim/pull/401))
