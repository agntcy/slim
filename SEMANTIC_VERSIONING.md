# Semantic Versioning Guidelines for SLIM

This document outlines the semantic versioning policy for the SLIM project
and defines what constitutes breaking changes, feature additions, and patch
releases.

## Overview

SLIM follows [Semantic Versioning 2.0.0](https://semver.org/) with the
format `MAJOR.MINOR.PATCH`:

- **MAJOR** version: Incompatible API changes or breaking changes
- **MINOR** version: New functionality added in a backwards-compatible
  manner (with caveats)
- **PATCH** version: Backwards-compatible bug fixes and improvements

## Version Compatibility Guarantees

### Major Version (X.0.0)

**Compatibility**: NOT backwards compatible

Major version upgrades indicate breaking changes that require user
intervention. Code written for version `N.x.x` is **not guaranteed** to work
with version `N+1.x.x` without modifications.

**Configuration files** may contain breaking changes and might **not be
backwards compatible**. Existing configuration files may need to be updated
to work with the new major version.

**Example**: Upgrading from `0.7.x` to `1.0.0` may require code changes.

### Minor Version (0.X.0)

**Compatibility**: Backwards compatible with limitations

Minor version upgrades maintain backwards compatibility at the **wire
protocol**, **configuration file** and **core API** level, but:

- New features may be introduced that are not available in older versions
- Applications built against version `0.N.x` can communicate with
  applications built against `0.M.x` (where M > N), but may not be able to
  use newer features
- Newer applications should gracefully handle the absence of features when
  communicating with older versions

**Configuration files** may contain new features but remain **backwards
compatible**. Existing configuration files from older minor versions will
continue to work with newer minor versions, though new optional
configuration options may be available.

**Example**: An application using SLIM `0.8.0` can communicate with an
application using SLIM `0.7.0`, but features introduced in `0.8.0` will not
be available when communicating with the `0.7.0` application.

### Patch Version (0.0.X)

**Compatibility**: Fully backwards compatible

Patch version upgrades are fully compatible. Applications can upgrade from
`0.N.P` to `0.N.Q` (where Q > P) without any code changes or compatibility
concerns.

**Configuration files** remain fully **backwards compatible**. No changes to
configuration files are required when upgrading patch versions.

**Example**: Upgrading from `0.7.1` to `0.7.4` requires no code changes and
maintains full compatibility.

## Breaking Changes Definition

The following changes constitute **MAJOR** version bumps:

### 1. Protocol Behavior Changes

Changes to the communication protocol behavior that break interoperability
between different versions. This applies to both:
- **Data Plane Protocol**: SLIM-to-SLIM data plane communication
- **Control Plane Protocol**: SLIM-to-controller communication

**Critical**: Protocol behavior changes that break interoperability
require a **MAJOR** version bump, **even if the public API remains
unchanged**. Loss of interoperability between versions is a breaking
change regardless of API stability.

**Important**: Wire compatibility (protobuf schema compatibility) is
**not sufficient** for protocol compatibility. Even if the protobuf
messages are wire-compatible, changes in protocol behavior can break
communication between versions.

**Breaking**:

- Changing message exchange patterns (e.g., adding required handshake
  messages, removing automatic acknowledgments)
- Changing when messages are sent (e.g., automatically sending subscription
  messages vs. requiring explicit requests)
- Changing message ordering requirements or timing expectations
- Changing required vs. optional message flows
- Changing state machine behavior or session lifecycle expectations
- Modifying error handling behavior that affects message flow
- Changing authentication or authorization flows

**Non-breaking** (can be MINOR):

- Adding optional message exchange patterns with graceful degradation
- Adding new optional features that older versions can safely ignore
- Performance optimizations that don't change message flows
- Internal state changes that don't affect external behavior

### 2. Protocol Buffer (Proto) Schema Changes

Changes to the proto files that break wire compatibility. This applies to
both:
- **Data Plane Proto**: `data-plane/core/datapath/proto/` (SLIM-to-SLIM
  data plane communication)
- **Control Plane Proto**: `proto/controller/v1/` (SLIM-to-controller
  communication)

**Note**: Wire-compatible protobuf changes are necessary but **not
sufficient** for protocol compatibility. See "Protocol Behavior Changes"
above.

**Breaking**:

- Removing fields from messages
- Changing field numbers
- Changing field types (e.g., `string` to `bytes`)
- Removing enum values that are in use
- Renaming messages or fields (unless field numbers are preserved)
- Changing service method signatures

**Non-breaking** (can be MINOR):

- Adding new optional fields
- Adding new enum values (with proper default handling)
- Adding new messages
- Adding new service methods
- Marking fields as deprecated (with migration path)

### 3. Public Binding APIs

Changes to the public APIs exposed by the Rust bindings in
`data-plane/bindings/rust/`.

**Note**: Language bindings (Python, Go, etc.) are **automatically
generated** from the Rust bindings using
[UniFFI](https://mozilla.github.io/uniffi-rs/). Therefore, any breaking
change to the Rust bindings API will propagate to all generated language
bindings.

#### Rust Bindings (`data-plane/bindings/rust/`)

The Rust bindings serve as the source of truth for all language bindings.
Focus on items marked with UniFFI attributes (`#[uniffi::export]`,
`#[derive(uniffi::Record)]`, `#[derive(uniffi::Object)]`, etc.).

**Breaking**:

- Removing public functions, structs, enums, or traits marked with UniFFI
  attributes
- Changing function signatures (parameter types, return types, or order)
- Changing struct field types in UniFFI records
- Removing enum variants from UniFFI enums
- Renaming public items exposed through UniFFI
- Changing error types in function return values
- Removing or changing methods on UniFFI objects

**Non-breaking** (can be MINOR):

- Adding new public functions, structs, enums, or traits with UniFFI
  attributes
- Adding new optional parameters with defaults
- Adding new enum variants (with proper default handling)
- Adding new methods to UniFFI objects
- Adding new struct fields (with defaults or optional fields)
- Adding new error variants (as long as existing ones remain)

#### Generated Language Bindings Impact

Since Python, Go, and other language bindings are generated from Rust:

- Breaking changes in Rust bindings automatically become breaking changes
  in **all** language bindings
- New Rust API additions automatically become available in **all** language
  bindings
- No separate versioning needed for individual language bindings - they
  track the Rust bindings version

### 4. Configuration Schema Changes

**Breaking**:

- Removing configuration options
- Changing required fields
- Changing configuration value formats or types
- Changing default behaviors that affect functionality

**Non-breaking** (can be MINOR):

- Adding new optional configuration fields
- Adding new configuration sections with sensible defaults
- Deprecating configuration options (with backwards compatibility)

## Non-Breaking Changes (MINOR version)

The following changes warrant a **MINOR** version bump:

1. **New Features**:
    - New session types or messaging patterns
    - New authentication methods
    - New configuration options (optional)
    - New API methods in bindings (additive only)

2. **Protocol Extensions**:
    - New optional proto fields
    - New message types
    - New service methods

3. **Performance Improvements**:
    - Optimizations that don't change behavior
    - New optional performance features

4. **Internal Refactoring**:
    - Changes to private/internal APIs
    - Dependency updates that don't affect public API

## Patch-Level Changes (PATCH version)

The following changes warrant a **PATCH** version bump:

1. **Bug Fixes**:
    - Fixes for incorrect behavior
    - Security patches
    - Memory leak fixes
    - Error handling improvements

2. **Documentation**:
    - Documentation updates
    - Code comment improvements
    - Example fixes

3. **Internal Changes**:
    - Test improvements
    - Build system updates
    - CI/CD improvements
    - Logging improvements

4. **Performance**:
    - Minor performance optimizations
    - Dependency updates (no API changes)

## Pre-1.0 Versioning

During the `0.x.x` phase (pre-1.0), the versioning strategy is slightly
relaxed:

- **MINOR** versions (`0.X.0`) may include breaking changes with proper
  migration documentation
- **PATCH** versions (`0.X.Y`) should remain backwards compatible
- Breaking changes will be clearly documented in CHANGELOG.md and release
  notes

Once SLIM reaches `1.0.0`, strict semantic versioning will be enforced.

## Version Synchronization Across Components

SLIM is a multi-component project with several versioned artifacts:

### Core Components

The following components define the versioning for SLIM:

- `agntcy-slim` - Defines the version of the SLIM node executable (the
  data plane)
- `agntcy-slim-bindings` - Defines the version of all language bindings
  (source for UniFFI-generated bindings)
- `control-plane` - Defines the version of the SLIM control plane server
- `slimctl` - Defines the version of the SLIM CLI tool

**Version Synchronization**: All core components listed above **must be
versioned together** and keep their versions in sync. Different versions
across these components are **not guaranteed** to be able to communicate
with each other. This ensures clear compatibility guarantees and prevents
confusion about which component versions can interoperate.

Other internal crates (e.g., `agntcy-slim-datapath`, `agntcy-slim-service`)
and modules follow internal versioning and are not used directly by end
users. However, these internal crates are still published on crates.io and
follow semantic versioning policies if they are used as dependencies by
external projects.

### Language Bindings

All language bindings are **generated from** `agntcy-slim-bindings` using
UniFFI and track its version:

- Python: `slim-bindings` (PyPI) - generated via UniFFI
- Go: Go module - generated via UniFFI
- Others as available - all generated from the same Rust source

The version of language binding packages should match the
`agntcy-slim-bindings` version they were generated from.

### Release Coordination

When breaking changes occur:

1. **Protocol behavior changes**: Trigger MAJOR version bump across all
   components due to loss of interoperability
2. **Proto schema changes**: Trigger MAJOR version bump across all
   components
3. **Binding API changes**: Trigger MAJOR version bump across **all
   components** including data plane, control plane, and all language
   bindings. This maintains version synchronization and avoids confusion
   about which versions can communicate with each other.
4. **Internal changes**: May only affect specific crates

## Migration Path Requirements

For every **MAJOR** version release, the following must be provided:

1. **Migration Guide**: Detailed documentation in CHANGELOG.md showing:
    - What changed
    - Why it changed
    - How to migrate existing code
    - Before/after code examples

2. **Deprecation Warnings**: When possible, deprecate features in a MINOR
   release before removing in MAJOR release

3. **Compatibility Table**: Clear matrix showing version compatibility
   across components

## Checking for Breaking Changes

Before releasing, reviewers should verify:

1. **Protocol Behavior**: Review changes in message exchange patterns,
   timing, ordering, and state machine behavior for both data plane and
   control plane communication
2. **Proto Files**: Compare `data-plane/core/datapath/proto/v1/data_plane.proto`
   and `proto/controller/v1/controller.proto` with previous versions
3. **Rust Bindings API**: Review changes in
   `data-plane/bindings/rust/src/lib.rs` and all public modules marked
   with UniFFI attributes
4. **Configuration**: Check `ServiceConfiguration` and related config
   structs
5. **Core Service**: Review `data-plane/core/service/src/lib.rs` public
   exports

## Additional Considerations

### Protocol Compatibility

Protocol compatibility encompasses both wire compatibility and behavioral
compatibility:

**Wire Protocol Compatibility** (protobuf schema):
- Communication between different SLIM versions
- Stored message formats
- Message serialization/deserialization

**Protocol Behavior Compatibility** (message flows and patterns):
- Message exchange sequences and timing
- Required vs. optional message flows
- State machine behavior and session lifecycles
- Client-server interaction patterns

Both aspects must be considered when evaluating breaking changes. A change
can be wire-compatible but behaviorally incompatible, which still
constitutes a breaking change.

### Dependency Versioning

- Updating major versions of key dependencies (e.g., gRPC, protobuf) may
  require MAJOR version bump
- Security updates to dependencies may warrant PATCH release even if they
  change dependency versions

### Security Releases

Security fixes may be backported to older MAJOR versions when feasible:

- Critical vulnerabilities: Backport to last 2 major versions
- Security patches released as PATCH versions

## Summary Table

| Change Type                | Version | Examples                |
| -------------------------- | ------- | ----------------------- |
| Remove proto fields        | MAJOR   | Deleting message fields |
| Change proto field types   | MAJOR   | `string` → `bytes`      |
| Remove public API          | MAJOR   | Delete binding function |
| Change function signature  | MAJOR   | Add required parameter  |
| Add proto field (optional) | MINOR   | New optional field      |
| Add new API method         | MINOR   | New binding function    |
| Add new feature            | MINOR   | New session type        |
| Fix bug                    | PATCH   | Correct error handling  |
| Update documentation       | PATCH   | README improvements     |
| Performance improvement    | PATCH   | Optimize internal code  |
