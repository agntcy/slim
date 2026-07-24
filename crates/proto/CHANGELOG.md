# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0](https://github.com/agntcy/slim/compare/slim-proto-v0.4.0...slim-proto-v0.5.0) - 2026-07-24

### Added

- increase post-quantum crypto coverage for wasm and add hybrid key exchange ([#1887](https://github.com/agntcy/slim/pull/1887))
- *(session)* add mls re-key on rejon ([#1875](https://github.com/agntcy/slim/pull/1875))

### Other

- rename group in domain ([#1891](https://github.com/agntcy/slim/pull/1891))

## [0.4.0](https://github.com/agntcy/slim/compare/slim-proto-v0.3.4...slim-proto-v0.4.0) - 2026-07-20

### Added

- *(session)* add close and rejoin functions ([#1873](https://github.com/agntcy/slim/pull/1873))
- *(session)* Heartbeat-Based Disconnection Detection with Epoch ([#1868](https://github.com/agntcy/slim/pull/1868))

## [0.3.4](https://github.com/agntcy/slim/compare/slim-proto-v0.3.3...slim-proto-v0.3.4) - 2026-07-20

### Other

- updated the following local packages: agntcy-slim-version, agntcy-slim-config

## [0.3.3](https://github.com/agntcy/slim/compare/slim-proto-v0.3.2...slim-proto-v0.3.3) - 2026-07-16

### Other

- updated the following local packages: agntcy-slim-version, agntcy-slim-config

## [0.3.2](https://github.com/agntcy/slim/compare/slim-proto-v0.3.1...slim-proto-v0.3.2) - 2026-07-16

### Other

- updated the following local packages: agntcy-slim-config

## [0.3.1](https://github.com/agntcy/slim/compare/slim-proto-v0.3.0...slim-proto-v0.3.1) - 2026-07-16

### Other

- updated the following local packages: agntcy-slim-config

## [0.3.0](https://github.com/agntcy/slim/compare/slim-proto-v0.2.0...slim-proto-v0.3.0) - 2026-07-15

### Added

- *(session)* use uuid for channel ids ([#1809](https://github.com/agntcy/slim/pull/1809))
- add websocket supports for the browser ([#1775](https://github.com/agntcy/slim/pull/1775))
- group registration via slimctl ([#1795](https://github.com/agntcy/slim/pull/1795))

### Fixed

- separate server connection config ([#1778](https://github.com/agntcy/slim/pull/1778))

## [0.2.0](https://github.com/agntcy/slim/compare/slim-proto-v0.1.0...slim-proto-v0.2.0) - 2026-07-06

### Added

- authenticate node group membership on registration ([#1782](https://github.com/agntcy/slim/pull/1782))

## [0.1.0](https://github.com/agntcy/slim/releases/tag/slim-proto-v0.1.0) - 2026-07-01

### Added

- config mode vs api mode for topology management ([#1772](https://github.com/agntcy/slim/pull/1772))
- add header integrity check and replay protection to control messages ([#1740](https://github.com/agntcy/slim/pull/1740))

### Other

- `slimctl` CLI commands for group-based routing ([#1769](https://github.com/agntcy/slim/pull/1769))
- restructure repo as pure Rust workspace ([#1693](https://github.com/agntcy/slim/pull/1693))
