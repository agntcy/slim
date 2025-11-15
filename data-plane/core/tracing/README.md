# SLIM Tracing Module

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)

The `agntcy-slim-tracing` module provides comprehensive observability for the
Agntcy SLIM data plane through structured logging, distributed tracing, and metrics. It
offers a flexible configuration system for controlling logging levels and
enabling OpenTelemetry integration.

## Overview

This module serves as the observability foundation for all SLIM components,
enabling:

- Structured logging for debugging and operational insights
- Distributed tracing to track requests across service boundaries
- Metrics collection for performance monitoring and alerts
- Integration with standard observability platforms

The tracing module uses [tracing](https://github.com/tokio-rs/tracing) and
[OpenTelemetry](https://opentelemetry.io/) for a unified approach to
observability, allowing developers to diagnose issues across the SLIM ecosystem.

## Logging Configuration

Logging behavior is controlled by three sources, in order of precedence (highest first):

1. Environment variable: RUST_LOG
2. Programmatic / file configuration: `TracingConfiguration { log_level, filters, ... }`
3. Built-in defaults: default `log_level = "info"` and default module `filters` list

### Base Concepts

- `log_level` (string): The default verbosity applied to all modules (unless overridden).
- `filters` (`Vec<String>`): A list of module directives. Each entry may be either:
  - A module name (e.g. `slim_service`)
  - A module with explicit level (e.g. `slim_service=debug`)
- Default `filters` contains only module names (no `=level`). They all inherit `log_level`.

### Precedence & Resolution Rules

1. If `RUST_LOG` is set it fully overrides the configuration.  
   - If it contains ONLY `module=level` directives (e.g. `slim=debug,slim_auth=trace`) and no global (bare) level, an implicit `,off` is appended internally so that all unspecified modules are silenced.
   - If it includes a bare/global level (e.g. `info` or `info,slim=debug`) it is used as-is.
2. If `RUST_LOG` is NOT set:
   - If the configured `filters` equals the default list, every module in that list is assigned the configured `log_level`.
   - If the configured `filters` differs from the default:
     - Entries with `=level` keep their specified level.
     - Entries without `=level` inherit the configured `log_level`.
3. A global fallback directive is always installed using `log_level` so that unmentioned modules do not become overly verbose.

### Examples

| Input Source | Value | Effective Behavior |
|--------------|-------|--------------------|
| (no env), defaults | log_level=info | All default slim* modules log at info |
| (no env), config.filters = ["slim_service", "slim_auth=trace"] and log_level=warn | `slim_service=warn`, `slim_auth=trace` |
| RUST_LOG=slim=debug | Augmented to `slim=debug,off` — only `slim` module at debug |
| RUST_LOG=slim=debug,slim_auth=trace | Augmented to `slim=debug,slim_auth=trace,off` |
| RUST_LOG=debug | Global debug level for all modules |
| RUST_LOG=info,slim=debug | All modules info, `slim` debug |
| RUST_LOG=slim=debug,other=info,slim_mls=trace | Only those three modules active; others off (implicit) |

### Rationale for Appending `,off`

When users specify only targeted `module=level` directives, they usually intend to restrict output. Appending `off` prevents unrelated third-party crates from emitting logs unexpectedly.

### Customization Tips

- To see everything quickly: `export RUST_LOG=trace`
- To focus on a single crate: `export RUST_LOG=slim_session=debug`
- To combine focused and general logging: `export RUST_LOG=warn,slim_auth=debug`

### Interaction With OpenTelemetry

Log filtering affects only emitted spans' events & formatted logs; OpenTelemetry spans are still created based on the sampler configuration. Adjust sampling separately if you need to reduce trace volume.

### Future Extensions (Potential)

- Wildcard module matching (e.g. `slim_*=debug`)
- Separate env var for default level (e.g. `SLIM_LOG_LEVEL`)
- Denylist support (`!module` syntax)

This behavior is implemented inside `TracingConfiguration::setup_tracing_subscriber`.
