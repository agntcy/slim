# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/agntcy/slim/compare/slim-mcp-proxy-v0.1.7...slim-mcp-proxy-v0.2.0) - 2025-10-09

### Added

- implement control plane group management ([#554](https://github.com/agntcy/slim/pull/554))
- *(session)* introduce session metadata ([#744](https://github.com/agntcy/slim/pull/744))
- [**breaking**] refactor session receive() API ([#731](https://github.com/agntcy/slim/pull/731))

### Other

- release ([#822](https://github.com/agntcy/slim/pull/822))
- *(service)* session files in separate module ([#695](https://github.com/agntcy/slim/pull/695))
- rename session types in rust code ([#679](https://github.com/agntcy/slim/pull/679))

## [0.1.7](https://github.com/agntcy/slim/compare/slim-mcp-proxy-v0.1.6...slim-mcp-proxy-v0.1.7) - 2025-09-18

### Fixed

- *(python-bindings)* default crypto provider initialization for Reqwest crate ([#706](https://github.com/agntcy/slim/pull/706))
- use duration-string in place of duration-str ([#683](https://github.com/agntcy/slim/pull/683))

### Other

- release dataplane v0.5.0 ([#508](https://github.com/agntcy/slim/pull/508))
- *(data-plane)* move to clang-19 ([#662](https://github.com/agntcy/slim/pull/662))
- *(python-bindings)* improve folder structure and Taskfile division ([#628](https://github.com/agntcy/slim/pull/628))

## [0.1.6](https://github.com/agntcy/slim/compare/slim-mcp-proxy-v0.1.5...slim-mcp-proxy-v0.1.6) - 2025-07-31

### Added

- add identity and mls options to python bindings ([#436](https://github.com/agntcy/slim/pull/436))
- add client connections to control plane ([#429](https://github.com/agntcy/slim/pull/429))
- implement MLS key rotation ([#412](https://github.com/agntcy/slim/pull/412))
- *(proto)* introduce SessionType in message header ([#410](https://github.com/agntcy/slim/pull/410))
- integrate MLS with auth ([#385](https://github.com/agntcy/slim/pull/385))
- *(control-plane)* handle all configuration parameters when creating a new connection ([#360](https://github.com/agntcy/slim/pull/360))
- add mls message types in slim messages ([#386](https://github.com/agntcy/slim/pull/386))
- add auth support in sessions ([#382](https://github.com/agntcy/slim/pull/382))
- channel creation in session layer ([#374](https://github.com/agntcy/slim/pull/374))
- support hot reload of TLS certificates ([#359](https://github.com/agntcy/slim/pull/359))
- *(config)* update the public/private key on file change ([#356](https://github.com/agntcy/slim/pull/356))
- *(config)* add watcher for file modifications ([#353](https://github.com/agntcy/slim/pull/353))
- *(auth)* jwt middleware ([#352](https://github.com/agntcy/slim/pull/352))
- derive name id from provided identity ([#345](https://github.com/agntcy/slim/pull/345))

### Fixed

- *(channel_endpoint)* extend mls for all sessions ([#411](https://github.com/agntcy/slim/pull/411))
- *(auth)* make simple identity usable for groups ([#387](https://github.com/agntcy/slim/pull/387))

### Other

- release ([#319](https://github.com/agntcy/slim/pull/319))
- remove Agent and AgentType and adopt Name as application identifier ([#477](https://github.com/agntcy/slim/pull/477))

## [0.1.5](https://github.com/agntcy/agp/compare/slim-mcp-proxy-v0.1.4...slim-mcp-proxy-v0.1.5) - 2025-06-03

### Fixed

- remove agntcy- prefix from mcp-proxy release ([#301](https://github.com/agntcy/agp/pull/301))

## [0.1.4](https://github.com/agntcy/slim/compare/slim-mcp-proxy-v0.1.3...slim-mcp-proxy-v0.1.4) - 2025-05-15

### Fixed

- properly close slim-mcp proxy ([#261](https://github.com/agntcy/slim/pull/261))

## [0.1.3](https://github.com/agntcy/slim/compare/slim-mcp-proxy-v0.1.2...slim-mcp-proxy-v0.1.3) - 2025-05-12

### Added

- add sse transport to the mcp-server-time example ([#234](https://github.com/agntcy/slim/pull/234))
- detect client disconnection in mcp proxy ([#208](https://github.com/agntcy/slim/pull/208))

### Other

- add integration test suite ([#233](https://github.com/agntcy/slim/pull/233))

## [0.1.2](https://github.com/agntcy/slim/compare/slim-mcp-proxy-v0.1.1...slim-mcp-proxy-v0.1.2) - 2025-05-06

### Added

- release mcp-proxy docker image ([#210](https://github.com/agntcy/slim/pull/210))

## [0.1.1](https://github.com/agntcy/slim/compare/slim-mcp-proxy-v0.1.0...slim-mcp-proxy-v0.1.1) - 2025-05-06

### Added

- release mcp proxy ([#206](https://github.com/agntcy/slim/pull/206))
