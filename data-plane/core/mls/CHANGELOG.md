# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/agntcy/slim/releases/tag/slim-mls-v0.1.0) - 2025-07-31

### Added

- add identity and mls options to python bindings ([#436](https://github.com/agntcy/slim/pull/436))
- implement key rotation proposal message exchange ([#434](https://github.com/agntcy/slim/pull/434))
- implement MLS key rotation ([#412](https://github.com/agntcy/slim/pull/412))
- integrate MLS with auth ([#385](https://github.com/agntcy/slim/pull/385))
- add mls message types in slim messages ([#386](https://github.com/agntcy/slim/pull/386))
- push and verify identities in message headers ([#384](https://github.com/agntcy/slim/pull/384))
- add the ability to drop messages from the interceptor ([#371](https://github.com/agntcy/slim/pull/371))
- implement MLS ([#307](https://github.com/agntcy/slim/pull/307))

### Other

- remove Agent and AgentType and adopt Name as application identifier ([#477](https://github.com/agntcy/slim/pull/477))
- add test application for dynamic MLS groups ([#435](https://github.com/agntcy/slim/pull/435))
- 397 remove endpoints in mls groups ([#413](https://github.com/agntcy/slim/pull/413))
- *(session)* use parking_lot to sync access to MlsState ([#401](https://github.com/agntcy/slim/pull/401))
