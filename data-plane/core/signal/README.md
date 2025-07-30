# SLIM-Signal Module

[![License](https://img.shields.io/badge/license-Apache_2.0-green.svg)](https://github.com/agntcy/slim/blob/main/LICENSE)

A small, cross-platform library for handling OS signals in SLIM applications.
This module provides a unified API for signal handling across both Unix and
Windows platforms, enabling graceful application shutdown.

## Overview

The SLIM-Signal module abstracts platform-specific signal handling, offering a
simple interface to wait for and react to OS termination signals. Key features
include:

- Cross-platform signal handling with unified API
- Support for Unix-specific signals (SIGINT, SIGTERM)
- Windows Ctrl+C signal support
- Integration with Tokio for asynchronous operation
- Tracing integration for debugging and monitoring
