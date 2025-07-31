# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/agntcy/slim/compare/slim-controller-v0.1.1...slim-controller-v0.2.0) - 2025-07-31

### Added

- control plane service & slimctl cp commands ([#388](https://github.com/agntcy/slim/pull/388))
- add client connections to control plane ([#429](https://github.com/agntcy/slim/pull/429))
- add node register call to proto ([#406](https://github.com/agntcy/slim/pull/406))
- *(proto)* introduce SessionType in message header ([#410](https://github.com/agntcy/slim/pull/410))
- *(control-plane)* handle all configuration parameters when creating a new connection ([#360](https://github.com/agntcy/slim/pull/360))

### Other

- remove Agent and AgentType and adopt Name as application identifier ([#477](https://github.com/agntcy/slim/pull/477))

## [0.1.1](https://github.com/agntcy/slim/compare/slim-controller-v0.1.0...slim-controller-v0.1.1) - 2025-05-14

### Other

- *(slim-controller)* release v0.1.0 ([#250](https://github.com/agntcy/slim/pull/250))

## [0.1.0](https://github.com/agntcy/slim/releases/tag/slim-controller-v0.1.0) - 2025-05-14

### Added

- implement control API ([#147](https://github.com/agntcy/slim/pull/147))

### Fixed

- *(datapath)* keep protobuf in crate ([#248](https://github.com/agntcy/slim/pull/248))

### Other

- add integration test suite ([#233](https://github.com/agntcy/slim/pull/233))
