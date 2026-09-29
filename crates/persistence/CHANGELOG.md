# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - semver correction

`0.1.1` moved to the `mls-rs` 0.56 / `mls-rs-core` 0.27 line as a **patch**
release, which a `0.x` patch bump promises is compatible with `0.1.0` (still
on `mls-rs` 0.54). It is not: the two lines are ABI-incompatible. A consumer
resolving fresh from crates.io could land on `persistence 0.1.1` together
with `agntcy-slim-mls 0.3.10` (which needs the 0.54 line), producing two
`mls-rs` trees in one graph and a build failure. See
[#2142](https://github.com/agntcy/slim/issues/2142).

This release has no code changes from `0.1.1` — it exists so the version
number correctly signals the breaking dependency change that `0.1.1` should
have carried.

## [0.1.1](https://github.com/agntcy/slim/compare/slim-persistence-v0.1.0...slim-persistence-v0.1.1) - 2026-09-17

### Other

- update Cargo.toml dependencies

## [0.1.0](https://github.com/agntcy/slim/releases/tag/slim-persistence-v0.1.0) - 2026-07-29

### Added

- *(session)* encrypted MLS + session state persistence and restore ([#1820](https://github.com/agntcy/slim/pull/1820))

### Fixed

- *(session)* remove record + MLS state + pool entry on close (both roles) ([#1902](https://github.com/agntcy/slim/pull/1902))
