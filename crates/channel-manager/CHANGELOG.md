# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [3.2.0](https://github.com/agntcy/slim/compare/slim-channel-manager-v3.1.0...slim-channel-manager-v3.2.0) - 2026-10-09

### Added

- *(config,channel-manager)* JWT did:key app identity ([#2203](https://github.com/agntcy/slim/pull/2203))

### Fixed

- *(session,rpc)* expose the caller's verified identity to slim-rpc handlers ([#2204](https://github.com/agntcy/slim/pull/2204))

## [3.1.0](https://github.com/agntcy/slim/compare/slim-channel-manager-v3.0.1...slim-channel-manager-v3.1.0) - 2026-10-09

### Added

- *(channel-manager)* identify callers by their mTLS client certificate ([#2190](https://github.com/agntcy/slim/pull/2190))
- *(slimctl,cmctl)* expose channel ownership, grants and TTL ([#2185](https://github.com/agntcy/slim/pull/2185))
- *(channel-manager)* expire channels created with a TTL ([#2184](https://github.com/agntcy/slim/pull/2184))
- *(channel-manager)* persist channel ownership and used grants ([#2183](https://github.com/agntcy/slim/pull/2183))
- *(channel-manager)* refuse replayed grants ([#2182](https://github.com/agntcy/slim/pull/2182))
- *(channel-manager)* ask the channel owner when no grant is presented ([#2181](https://github.com/agntcy/slim/pull/2181))
- *(channel-manager)* signed-grant contract for AddParticipant/DeleteParticipant ([#2180](https://github.com/agntcy/slim/pull/2180))
- *(channel-manager)* channel ownership (owner principal per channel) ([#2178](https://github.com/agntcy/slim/pull/2178))
- *(channel-manager)* thread caller identity into RPC handlers ([#2177](https://github.com/agntcy/slim/pull/2177))

### Other

- *(channel-manager)* end-to-end tests for owned channels; fix owner approval ([#2192](https://github.com/agntcy/slim/pull/2192))
- release ([#2153](https://github.com/agntcy/slim/pull/2153))

## [3.0.2](https://github.com/agntcy/slim/compare/slim-channel-manager-v3.0.1...slim-channel-manager-v3.0.2) - 2026-09-30

### Other

- updated the following local packages: agntcy-slim-auth, agntcy-slim-config, agntcy-slim-proto, agntcy-slim-tracing, agntcy-slim-datapath, agntcy-slim-session, agntcy-slim-service, agntcy-slim

## [3.0.0](https://github.com/agntcy/slim/compare/slim-channel-manager-v2.3.3...slim-channel-manager-v3.0.0) - 2026-09-29

### Other

- remove redundant caller routes for default gateway ([#2097](https://github.com/agntcy/slim/pull/2097))

## [2.3.3](https://github.com/agntcy/slim/compare/slim-channel-manager-v2.3.0...slim-channel-manager-v2.3.3) - 2026-09-17

### Other

- release ([#2021](https://github.com/agntcy/slim/pull/2021))
- release ([#1992](https://github.com/agntcy/slim/pull/1992))

## [2.3.2](https://github.com/agntcy/slim/compare/slim-channel-manager-v2.3.0...slim-channel-manager-v2.3.2) - 2026-09-01

### Other

- release ([#1992](https://github.com/agntcy/slim/pull/1992))

## [2.3.1](https://github.com/agntcy/slim/compare/slim-channel-manager-v2.3.0...slim-channel-manager-v2.3.1) - 2026-09-01

### Other

- update Cargo.lock dependencies

## [2.0.0](https://github.com/agntcy/slim/compare/slim-channel-manager-v2.0.0...slim-channel-manager-v2.0.0) - 2026-08-04

### Other

- update Cargo.lock dependencies

## [2.0.0-alpha.11](https://github.com/agntcy/slim/compare/slim-channel-manager-v2.0.0-alpha.10...slim-channel-manager-v2.0.0-alpha.11) - 2026-08-03

### Other

- updated the following local packages: agntcy-slim-config, agntcy-slim-config, agntcy-slim-proto, agntcy-slim-tracing, agntcy-slim-datapath, agntcy-slim-datapath, agntcy-slim-session, agntcy-slim-service, agntcy-slim

## [2.0.0-alpha.8](https://github.com/agntcy/slim/compare/slim-channel-manager-v2.0.0-alpha.7...slim-channel-manager-v2.0.0-alpha.8) - 2026-07-29

### Added

- *(channel-manager)* add storage ([#1901](https://github.com/agntcy/slim/pull/1901))
- *(bindings)* expose session close/rejoin ([#1896](https://github.com/agntcy/slim/pull/1896))

## [2.0.0-alpha.5](https://github.com/agntcy/slim/compare/slim-channel-manager-v2.0.0-alpha.4...slim-channel-manager-v2.0.0-alpha.5) - 2026-07-16

### Other

- updated the following local packages: agntcy-slim-config, agntcy-slim-config, agntcy-slim-proto, agntcy-slim-tracing, agntcy-slim-datapath, agntcy-slim-datapath, agntcy-slim-session, agntcy-slim-service, agntcy-slim

## [2.0.0-alpha.4](https://github.com/agntcy/slim/compare/slim-channel-manager-v2.0.0-alpha.3...slim-channel-manager-v2.0.0-alpha.4) - 2026-07-16

### Other

- updated the following local packages: agntcy-slim-config, agntcy-slim-config, agntcy-slim-proto, agntcy-slim-tracing, agntcy-slim-datapath, agntcy-slim-datapath, agntcy-slim-session, agntcy-slim-service, agntcy-slim

## [2.0.0-alpha.3](https://github.com/agntcy/slim/compare/slim-channel-manager-v2.0.0-alpha.2...slim-channel-manager-v2.0.0-alpha.3) - 2026-07-15

### Other

- update Cargo.lock dependencies

## [2.0.0-alpha.2](https://github.com/agntcy/slim/releases/tag/slim-channel-manager-v2.0.0-alpha.2) - 2026-07-06

### Added

- authenticate node group membership on registration ([#1782](https://github.com/agntcy/slim/pull/1782))
- add header integrity check and replay protection to control messages ([#1740](https://github.com/agntcy/slim/pull/1740))

### Other

- align channel manager version with workspace ([#1798](https://github.com/agntcy/slim/pull/1798))
- restructure repo as pure Rust workspace ([#1693](https://github.com/agntcy/slim/pull/1693))
