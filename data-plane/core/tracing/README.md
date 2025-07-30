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
