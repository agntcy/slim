# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [2.3.3](https://github.com/agntcy/slim/compare/slim-control-plane-v2.3.0...slim-control-plane-v2.3.3) - 2026-09-03

### Fixed

- *(control-plane)* serialize node_registered with deregister/disconnect ([#1997](https://github.com/agntcy/slim/pull/1997))

### Other

- release ([#2021](https://github.com/agntcy/slim/pull/2021))
- release ([#1992](https://github.com/agntcy/slim/pull/1992))

## [2.3.2](https://github.com/agntcy/slim/compare/slim-control-plane-v2.3.0...slim-control-plane-v2.3.2) - 2026-09-01

### Fixed

- *(control-plane)* serialize node_registered with deregister/disconnect ([#1997](https://github.com/agntcy/slim/pull/1997))

### Other

- release ([#1992](https://github.com/agntcy/slim/pull/1992))

## [2.3.1](https://github.com/agntcy/slim/compare/slim-control-plane-v2.3.0...slim-control-plane-v2.3.1) - 2026-09-01

### Fixed

- *(control-plane)* serialize node_registered with deregister/disconnect ([#1997](https://github.com/agntcy/slim/pull/1997))

## [2.3.0](https://github.com/agntcy/slim/compare/slim-control-plane-v2.2.0...slim-control-plane-v2.3.0) - 2026-08-13

### Fixed

- propagate OIDC auth through merge and diff_connections ([#1983](https://github.com/agntcy/slim/pull/1983))

## [2.1.1](https://github.com/agntcy/slim/compare/slim-control-plane-v2.1.0...slim-control-plane-v2.1.1) - 2026-08-12

### Fixed

- windows build ([#1978](https://github.com/agntcy/slim/pull/1978))

## [2.1.0](https://github.com/agntcy/slim/compare/slim-control-plane-v2.0.0...slim-control-plane-v2.1.0) - 2026-08-12

### Added

- *(control-plane)* support multiple northbound and southbound listeners ([#1966](https://github.com/agntcy/slim/pull/1966))

## [2.0.0](https://github.com/agntcy/slim/compare/slim-control-plane-v2.0.0...slim-control-plane-v2.0.0) - 2026-08-04

### Other

- update Cargo.lock dependencies

## [2.0.0-alpha.11](https://github.com/agntcy/slim/compare/slim-control-plane-v2.0.0-alpha.10...slim-control-plane-v2.0.0-alpha.11) - 2026-08-03

### Other

- updated the following local packages: agntcy-slim-config, agntcy-slim-config, agntcy-slim-proto, agntcy-slim-tracing, agntcy-slim-tracing, agntcy-slim-datapath, agntcy-slim-datapath, agntcy-slim-service

## [2.0.0-alpha.9](https://github.com/agntcy/slim/compare/slim-control-plane-v2.0.0-alpha.8...slim-control-plane-v2.0.0-alpha.9) - 2026-07-31

### Added

- add control-plane side override of node connection data ([#1913](https://github.com/agntcy/slim/pull/1913))

## [2.0.0-alpha.8](https://github.com/agntcy/slim/compare/slim-control-plane-v2.0.0-alpha.7...slim-control-plane-v2.0.0-alpha.8) - 2026-07-29

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
