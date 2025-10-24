# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/agntcy/slim/compare/slim-session-v0.1.0...slim-session-v0.1.1) - 2025-10-24

### Added

- async mls ([#877](https://github.com/agntcy/slim/pull/877))
- expand SharedSecret Auth from simple secret:id to HMAC tokens ([#858](https://github.com/agntcy/slim/pull/858))
- derive name ID part from identity token ([#851](https://github.com/agntcy/slim/pull/851))x
- *(session)* create sender and receiver ([#836](https://github.com/agntcy/slim/pull/836))

### Other

- split mls state and channel endpoint ([#875](https://github.com/agntcy/slim/pull/875))
- implement all control message payload in protobuf ([#862](https://github.com/agntcy/slim/pull/862))
- *(agntcy-slim-session)* release v0.1.0 ([#856](https://github.com/agntcy/slim/pull/856))

## [0.1.0](https://github.com/agntcy/slim/releases/tag/slim-session-v0.1.0) - 2025-10-17

### Added

- move session code in a new crate ([#828](https://github.com/agntcy/slim/pull/828))

### Fixed

- *(session)* correctly handle multiple subscriptions ([#838](https://github.com/agntcy/slim/pull/838))
