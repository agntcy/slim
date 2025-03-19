# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.1](https://github.com/agntcy/agp/compare/agp-datapath-v0.4.0...agp-datapath-v0.4.1) - 2025-03-19

### Added

- *(tables)* do not require Default/Clone traits for elements stored in pool ([#97](https://github.com/agntcy/agp/pull/97))

### Other

- use same API for send_to and publish ([#89](https://github.com/agntcy/agp/pull/89))

## [0.4.0](https://github.com/agntcy/agp/compare/agp-datapath-v0.3.1...agp-datapath-v0.4.0) - 2025-03-18

### Added

- new message format ([#88](https://github.com/agntcy/agp/pull/88))

## [0.3.1](https://github.com/agntcy/agp/compare/agp-datapath-v0.3.0...agp-datapath-v0.3.1) - 2025-03-18

### Added

- propagate context to enable distributed tracing ([#90](https://github.com/agntcy/agp/pull/90))

## [0.3.0](https://github.com/agntcy/agp/compare/agp-datapath-v0.2.1...agp-datapath-v0.3.0) - 2025-03-12

### Added

- notify local app if a message is not processed correctly ([#72](https://github.com/agntcy/agp/pull/72))

## [0.2.1](https://github.com/agntcy/agp/compare/agp-datapath-v0.2.0...agp-datapath-v0.2.1) - 2025-03-11

### Other

- *(agp-config)* release v0.1.4 ([#79](https://github.com/agntcy/agp/pull/79))

## [0.2.0](https://github.com/agntcy/agp/compare/agp-datapath-v0.1.2...agp-datapath-v0.2.0) - 2025-02-28

### Added

- handle disconnection events (#67)

## [0.1.2](https://github.com/agntcy/agp/compare/agp-datapath-v0.1.1...agp-datapath-v0.1.2) - 2025-02-28

### Added

- add message handling metrics

## [0.1.1](https://github.com/agntcy/agp/compare/agp-datapath-v0.1.0...agp-datapath-v0.1.1) - 2025-02-19

### Added

- *(tables)* distinguish local and remote connections in the subscription table (#55)

## [0.1.0](https://github.com/agntcy/agp/releases/tag/agp-gw-data-path-v0.1.0) - 2025-02-09

### Added

- Stage the first commit of the agent gateway protocol (#3)

### Other

- release process for rust crates
