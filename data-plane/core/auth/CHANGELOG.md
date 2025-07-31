# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
