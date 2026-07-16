# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
