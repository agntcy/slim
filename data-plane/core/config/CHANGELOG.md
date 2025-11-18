# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.2](https://github.com/agntcy/slim/compare/slim-config-v0.4.1...slim-config-v0.4.2) - 2025-11-17

### Added

- add async initialize func in the provider/verifier traits ([#917](https://github.com/agntcy/slim/pull/917))
- add backoff retry ([#939](https://github.com/agntcy/slim/pull/939))
- Integrate SPIRE-based mTLS & identity, unify TLS sources, enhance gRPC config, and add flexible metadata support ([#892](https://github.com/agntcy/slim/pull/892))
- *(auth)* add support for setting custom claims while getting the token ([#879](https://github.com/agntcy/slim/pull/879))
- implementation of Spire for fetching the certificates/token directly from SPIFFE Workload API ([#646](https://github.com/agntcy/slim/pull/646))

### Fixed

- *(spire)* get all x509 bundles ([#960](https://github.com/agntcy/slim/pull/960))

### Other

- unify multicast and P2P session handling ([#904](https://github.com/agntcy/slim/pull/904))
- *(data-plane)* update project dependencies ([#861](https://github.com/agntcy/slim/pull/861))

## [0.4.1](https://github.com/agntcy/slim/compare/slim-config-v0.4.0...slim-config-v0.4.1) - 2025-10-17

### Added

- implementation of Identity provider client credential flow ([#464](https://github.com/agntcy/slim/pull/464))

## [0.4.0](https://github.com/agntcy/slim/compare/slim-config-v0.3.0...slim-config-v0.4.0) - 2025-10-09

### Added

- implement control plane group management ([#554](https://github.com/agntcy/slim/pull/554))
- remove bearer auth in favour of static jwt ([#774](https://github.com/agntcy/slim/pull/774))

### Fixed

- load all certificates for dataplane from ca ([#772](https://github.com/agntcy/slim/pull/772))

## [0.3.0](https://github.com/agntcy/slim/compare/slim-config-v0.2.0...slim-config-v0.3.0) - 2025-09-17

### Added

- add metadata map to clients and servers ([#684](https://github.com/agntcy/slim/pull/684))
- *(grpc-client)* add support for HTTPS proxy ([#614](https://github.com/agntcy/slim/pull/614))
- notify controller with new subscriptions ([#611](https://github.com/agntcy/slim/pull/611))
- *(grpc)* add support for HTTP proxy ([#610](https://github.com/agntcy/slim/pull/610))

### Fixed

- *(python-bindings)* default crypto provider initialization for Reqwest crate ([#706](https://github.com/agntcy/slim/pull/706))
- use duration-string in place of duration-str ([#683](https://github.com/agntcy/slim/pull/683))
- *(tls)* enable loading of system ca certs by default ([#605](https://github.com/agntcy/slim/pull/605))

### Other

- SLIM node ID should be unique in a deployment ([#630](https://github.com/agntcy/slim/pull/630))

## [0.2.0](https://github.com/agntcy/slim/compare/slim-config-v0.1.8...slim-config-v0.2.0) - 2025-07-31

### Added

- *(auth)* support JWK as decoding keys ([#461](https://github.com/agntcy/slim/pull/461))
- add client connections to control plane ([#429](https://github.com/agntcy/slim/pull/429))
- *(control-plane)* handle all configuration parameters when creating a new connection ([#360](https://github.com/agntcy/slim/pull/360))
- support hot reload of TLS certificates ([#359](https://github.com/agntcy/slim/pull/359))
- *(config)* update the public/private key on file change ([#356](https://github.com/agntcy/slim/pull/356))
- *(auth)* introduce token provider trait ([#357](https://github.com/agntcy/slim/pull/357))
- *(config)* add watcher for file modifications ([#353](https://github.com/agntcy/slim/pull/353))
- *(auth)* jwt middleware ([#352](https://github.com/agntcy/slim/pull/352))

### Other

- remove Agent and AgentType and adopt Name as application identifier ([#477](https://github.com/agntcy/slim/pull/477))

## [0.1.8](https://github.com/agntcy/slim/compare/slim-config-v0.1.7...slim-config-v0.1.8) - 2025-05-14

### Fixed

- add the possibility to ignore spelling errors ([#199](https://github.com/agntcy/slim/pull/199))

## [0.1.7](https://github.com/agntcy/slim/compare/slim-config-v0.1.6...slim-config-v0.1.7) - 2025-04-24

### Added

- improve configuration handling for tracing ([#186](https://github.com/agntcy/slim/pull/186))
- *(data-plane)* support for multiple servers ([#173](https://github.com/agntcy/slim/pull/173))

### Fixed

- *(data-plane)* make new linter version happy ([#184](https://github.com/agntcy/slim/pull/184))

### Other

- declare all dependencies in workspace Cargo.toml ([#187](https://github.com/agntcy/slim/pull/187))
- *(data-plane)* tonic 0.12.3 -> 0.13 ([#170](https://github.com/agntcy/slim/pull/170))
- upgrade to rust edition 2024 and toolchain 1.86.0 ([#164](https://github.com/agntcy/slim/pull/164))

## [0.1.6](https://github.com/agntcy/slim/compare/slim-config-v0.1.5...slim-config-v0.1.6) - 2025-04-08

### Other

- update copyright ([#109](https://github.com/agntcy/slim/pull/109))

## [0.1.5](https://github.com/agntcy/slim/compare/slim-config-v0.1.4...slim-config-v0.1.5) - 2025-03-18

### Added

- propagate context to enable distributed tracing ([#90](https://github.com/agntcy/slim/pull/90))

## [0.1.4](https://github.com/agntcy/slim/compare/slim-config-v0.1.3...slim-config-v0.1.4) - 2025-03-11

### Other

- *(slim-config)* release v0.1.3 ([#75](https://github.com/agntcy/slim/pull/75))

## [0.1.3](https://github.com/agntcy/slim/compare/slim-config-v0.1.2...slim-config-v0.1.3) - 2025-03-07

### Other

- Windows Instructions ([#73](https://github.com/agntcy/slim/pull/73))

## [0.1.2](https://github.com/agntcy/slim/compare/slim-config-v0.1.1...slim-config-v0.1.2) - 2025-02-28

### Added

- add message handling metrics

## [0.1.1](https://github.com/agntcy/slim/compare/slim-config-v0.1.0...slim-config-v0.1.1) - 2025-02-14

### Other

- updated the following local packages: slim-tracing

## [0.1.0](https://github.com/agntcy/slim/releases/tag/slim-config-v0.1.0) - 2025-02-10

### Other

- reduce the number of crates to publish (#10)
