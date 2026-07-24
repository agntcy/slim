# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [2.0.0-alpha.8](https://github.com/agntcy/slim/compare/slim-control-plane-v2.0.0-alpha.7...slim-control-plane-v2.0.0-alpha.8) - 2026-07-24

### Fixed

- link recreation after node restart ([#1898](https://github.com/agntcy/slim/pull/1898))

### Other

- rename group in domain ([#1891](https://github.com/agntcy/slim/pull/1891))

## [2.0.0-alpha.5](https://github.com/agntcy/slim/compare/slim-control-plane-v2.0.0-alpha.4...slim-control-plane-v2.0.0-alpha.5) - 2026-07-16

### Other

- updated the following local packages: agntcy-slim-config, agntcy-slim-config, agntcy-slim-proto, agntcy-slim-tracing, agntcy-slim-tracing, agntcy-slim-datapath, agntcy-slim-datapath, agntcy-slim-service

## [2.0.0-alpha.4](https://github.com/agntcy/slim/compare/slim-control-plane-v2.0.0-alpha.3...slim-control-plane-v2.0.0-alpha.4) - 2026-07-16

### Other

- updated the following local packages: agntcy-slim-config, agntcy-slim-config, agntcy-slim-proto, agntcy-slim-tracing, agntcy-slim-tracing, agntcy-slim-datapath, agntcy-slim-datapath, agntcy-slim-service

## [2.0.0-alpha.3](https://github.com/agntcy/slim/compare/slim-control-plane-v2.0.0-alpha.2...slim-control-plane-v2.0.0-alpha.3) - 2026-07-15

### Added

- group registration via slimctl ([#1795](https://github.com/agntcy/slim/pull/1795))

### Fixed

- *(control-plane)* re-expand routes when a node reconnects over a claimed link ([#1836](https://github.com/agntcy/slim/pull/1836))
- separate server connection config ([#1778](https://github.com/agntcy/slim/pull/1778))

### Other

- *(control-plane)* fix auth-link flake by making the connector dialer-only ([#1831](https://github.com/agntcy/slim/pull/1831))
- *(control-plane)* de-flake integration tests ([#1829](https://github.com/agntcy/slim/pull/1829))

## [2.0.0-alpha.2](https://github.com/agntcy/slim/releases/tag/slim-control-plane-v2.0.0-alpha.2) - 2026-07-01

### Added

- config mode vs api mode for topology management ([#1772](https://github.com/agntcy/slim/pull/1772))
- network segmentation ([#1761](https://github.com/agntcy/slim/pull/1761))

### Other

- release 2.0.0 alpha 2 ([#1783](https://github.com/agntcy/slim/pull/1783))
- `slimctl` CLI commands for group-based routing ([#1769](https://github.com/agntcy/slim/pull/1769))
- restructure repo as pure Rust workspace ([#1693](https://github.com/agntcy/slim/pull/1693))
