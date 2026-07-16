# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [2.0.0-alpha.4](https://github.com/agntcy/slim/compare/slimctl-v2.0.0-alpha.3...slimctl-v2.0.0-alpha.4) - 2026-07-16

### Other

- updated the following local packages: agntcy-slim-config, agntcy-slim-proto, agntcy-slim-tracing, agntcy-slim-datapath, agntcy-slim-session, agntcy-slim-service, agntcy-slim

## [2.0.0-alpha.3](https://github.com/agntcy/slim/compare/slimctl-v2.0.0-alpha.2...slimctl-v2.0.0-alpha.3) - 2026-07-06

### Other

- updated the following local packages: agntcy-slim-config, agntcy-slim-proto, agntcy-slim-session, agntcy-slim-service, agntcy-slim-tracing, agntcy-slim-datapath, agntcy-slim

## [2.0.0-alpha.2](https://github.com/agntcy/slim/compare/slimctl-v2.0.0-alpha.1...slimctl-v2.0.0-alpha.2) - 2026-07-01

### Added

- config mode vs api mode for topology management ([#1772](https://github.com/agntcy/slim/pull/1772))
- *(slimctl)* improve bench pub/sub commands and reporting ([#1748](https://github.com/agntcy/slim/pull/1748))

### Other

- `slimctl` CLI commands for group-based routing ([#1769](https://github.com/agntcy/slim/pull/1769))
- restructure repo as pure Rust workspace ([#1693](https://github.com/agntcy/slim/pull/1693))

## [2.0.0-alpha.1](https://github.com/agntcy/slim/compare/slimctl-v2.0.0-alpha.0...slimctl-v2.0.0-alpha.1) - 2026-06-17

### Added

- *(websocket)* Enable the compilation of data-plane for wasm32 ([#1695](https://github.com/agntcy/slim/pull/1695))
- *(peer-sync)* subscription synchronization ([#1705](https://github.com/agntcy/slim/pull/1705))
- e2e header integrity validation ([#1677](https://github.com/agntcy/slim/pull/1677))

## [2.0.0-alpha.0](https://github.com/agntcy/slim/compare/slimctl-v1.4.0...slimctl-v2.0.0-alpha.0) - 2026-06-03

### Added

- increase Name ID from u64 to u128 ([#1680](https://github.com/agntcy/slim/pull/1680))
- *(slimctl)* add bench sub/pub and channel sub/pub commands ([#1602](https://github.com/agntcy/slim/pull/1602))
- *(websocket)* add WebSocket transport for data-plane ([#1638](https://github.com/agntcy/slim/pull/1638))
- *(dataplane)* remove group creation ([#1594](https://github.com/agntcy/slim/pull/1594))
- *(slimctl)* use channel manager to create groups ([#1589](https://github.com/agntcy/slim/pull/1589))
- *(slimctl)* add commands for channel-manager ([#1586](https://github.com/agntcy/slim/pull/1586))

### Fixed

- *(bindings)* deadlock when using single thread runtime ([#1657](https://github.com/agntcy/slim/pull/1657))

### Other

- publish alpha release 2.0.0-alpha.0 ([#1704](https://github.com/agntcy/slim/pull/1704))
- remove agntcy-slim-bindings dependency from slimctl ([#1700](https://github.com/agntcy/slim/pull/1700))
- control plane in rust ([#1581](https://github.com/agntcy/slim/pull/1581))
- *(data-plane)* replace encoder::Name with ProtoName throughout ([#1596](https://github.com/agntcy/slim/pull/1596))

## [1.4.0](https://github.com/agntcy/slim/compare/slimctl-v1.3.0...slimctl-v1.4.0) - 2026-04-21

### Added

- update controller connection ([#1485](https://github.com/agntcy/slim/pull/1485))

### Fixed

- *(slimctl)* correct test name spelling (writable) ([#1464](https://github.com/agntcy/slim/pull/1464))

### Other

- prepare 1.4.0 ([#1530](https://github.com/agntcy/slim/pull/1530))

## [1.3.0](https://github.com/agntcy/slim/compare/slimctl-v1.2.0...slimctl-v1.3.0) - 2026-03-31

### Added

- add agntcy-slim-version crate as single source of truth for version and build info ([#1360](https://github.com/agntcy/slim/pull/1360))

## [1.2.0](https://github.com/agntcy/slim/releases/tag/slimctl-v1.2.0) - 2026-02-27

### Added

- remove go implementation of slimctl and refactor workflows ([#1276](https://github.com/agntcy/slim/pull/1276))
- *(data-plane)* port slimctl to Rust ([#1255](https://github.com/agntcy/slim/pull/1255))

### Fixed

- bump slimctl version ([#1286](https://github.com/agntcy/slim/pull/1286))
