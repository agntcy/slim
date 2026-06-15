// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! # Channel Manager for SLIM
//!
//! A standalone Rust application that manages SLIM channels and participants.
//! Provides a gRPC API for dynamic channel and participant management, and
//! supports initial channel setup from a YAML configuration file.

// TODO(wasm32): channel-manager wraps tonic gRPC + the native session layer.
#![cfg(not(target_arch = "wasm32"))]

pub mod config;
pub mod service;
pub mod sessions;

pub mod proto {
    include!("gen/channel_manager.proto.v1.rs");
}
