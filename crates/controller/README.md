# SLIM Controller Module

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)

The `agntcy-slim-controller` module provides the control API and service for
configuring and managing the Agntcy SLIM data plane through the control plane. It
enables dynamic configuration of routing, connections, and subscriptions via a
bidirectional gRPC streaming interface.

## Overview

This module serves as the management layer for SLIM, allowing control plane
components (like `slimctl`) to configure and monitor SLIM data plane instances
at runtime. Key functionalities include:

- Establishing bidirectional control channels between control and data planes
- Dynamic management of connections and routes
- Subscription handling and configuration
- Real-time monitoring of SLIM components
- Status reporting and acknowledgment mechanisms
