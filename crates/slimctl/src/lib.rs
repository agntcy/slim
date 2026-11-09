// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Parsers behind `slimctl`'s command-line and config-file input, exposed so
//! the fuzz targets in `fuzz/` can call them. The `slimctl` binary does not
//! depend on this library.

// Shared with the binary, which uses `pub(crate)` items this library does not.
#[allow(dead_code)]
pub mod config;
pub mod proto;
pub mod utils;
