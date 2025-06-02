# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.16](https://github.com/agntcy/agp/compare/agp-gw-v0.3.15...agp-gw-v0.3.16) - 2025-06-02

### Added

- *(fire-and-forget)* add support for sticky sessions ([#281](https://github.com/agntcy/agp/pull/281))

## [0.3.15](https://github.com/agntcy/agp/compare/agp-gw-v0.3.14...agp-gw-v0.3.15) - 2025-05-14

### Fixed

- *(mcp-proxy)* dependencies ([#252](https://github.com/agntcy/agp/pull/252))

## [0.3.14](https://github.com/agntcy/agp/compare/agp-gw-v0.3.13...agp-gw-v0.3.14) - 2025-05-14

### Other

- updated the following local packages: agp-service

## [0.3.13](https://github.com/agntcy/agp/compare/agp-gw-v0.3.12...agp-gw-v0.3.13) - 2025-05-14

### Other

- updated the following local packages: agp-config, agp-service, agp-tracing

## [0.3.12](https://github.com/agntcy/agp/compare/agp-gw-v0.3.11...agp-gw-v0.3.12) - 2025-04-24

### Added

- *(python-bindings)* improve configuration handling and further refactoring ([#167](https://github.com/agntcy/agp/pull/167))
- *(data-plane)* support for multiple servers ([#173](https://github.com/agntcy/agp/pull/173))

### Other

- declare all dependencies in workspace Cargo.toml ([#187](https://github.com/agntcy/agp/pull/187))
- upgrade to rust edition 2024 and toolchain 1.86.0 ([#164](https://github.com/agntcy/agp/pull/164))

## [0.3.11](https://github.com/agntcy/agp/compare/agp-gw-v0.3.10...agp-gw-v0.3.11) - 2025-04-08

### Other

- update copyright ([#109](https://github.com/agntcy/agp/pull/109))

## [0.3.10](https://github.com/agntcy/agp/compare/agp-gw-v0.3.9...agp-gw-v0.3.10) - 2025-03-19

### Other

- release ([#103](https://github.com/agntcy/agp/pull/103))

## [0.3.9](https://github.com/agntcy/agp/compare/agp-gw-v0.3.8...agp-gw-v0.3.9) - 2025-03-19

### Other

- updated the following local packages: agp-service

## [0.3.8](https://github.com/agntcy/agp/compare/agp-gw-v0.3.7...agp-gw-v0.3.8) - 2025-03-18

### Other

- updated the following local packages: agp-service

## [0.3.7](https://github.com/agntcy/agp/compare/agp-gw-v0.3.6...agp-gw-v0.3.7) - 2025-03-18

### Other

- update Cargo.lock dependencies

## [0.3.6](https://github.com/agntcy/agp/compare/agp-gw-v0.3.5...agp-gw-v0.3.6) - 2025-03-12

### Other

- updated the following local packages: agp-service

## [0.3.5](https://github.com/agntcy/agp/compare/agp-gw-v0.3.4...agp-gw-v0.3.5) - 2025-03-11

### Other

- *(agp-config)* release v0.1.4 ([#79](https://github.com/agntcy/agp/pull/79))

## [0.3.4](https://github.com/agntcy/agp/compare/agp-gw-v0.3.3...agp-gw-v0.3.4) - 2025-02-28

### Other

- updated the following local packages: agp-service

## [0.3.3](https://github.com/agntcy/agp/compare/agp-gw-v0.3.2...agp-gw-v0.3.3) - 2025-02-28

### Added

- add message handling metrics

## [0.3.2](https://github.com/agntcy/agp/compare/agp-gw-v0.3.1...agp-gw-v0.3.2) - 2025-02-24

### Other

- update Cargo.lock dependencies

## [0.3.1](https://github.com/agntcy/agp/compare/agp-gw-v0.3.0...agp-gw-v0.3.1) - 2025-02-20

### Other

- release (#58)

## [0.3.0](https://github.com/agntcy/agp/compare/agp-gw-v0.2.1...agp-gw-v0.3.0) - 2025-02-14

### Added

- add build info to main gateway executable (#35)

## [0.2.1](https://github.com/agntcy/agp/compare/agp-gw-v0.2.0...agp-gw-v0.2.1) - 2025-02-14

### Added

- implement opentelemetry tracing subscriber

## [0.2.0](https://github.com/agntcy/agp/compare/agp-gw-v0.1.0...agp-gw-v0.2.0) - 2025-02-12

### Fixed

- *(gateway)* remove unused log level (#28)

## [0.1.0](https://github.com/agntcy/agp/releases/tag/agp-gw-v0.1.0) - 2025-02-10

### Added

- Stage the first commit of the agent gateway protocol (#3)

### Other

- reduce the number of crates to publish (#10)
