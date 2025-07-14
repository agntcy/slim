// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Command line argument parsing for AuthZEN integration example

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "authzen-demo",
    about = "SLIM AuthZEN Integration Demonstration",
    long_about = "
This example demonstrates SLIM's integration with the OpenID AuthZEN standard 
for fine-grained authorization. It shows how to configure and use AuthZEN for 
route creation, message publishing, and subscription authorization.

The example tests various authorization scenarios including:
- Agent creation and route establishment
- Publish authorization with message size limits
- Subscribe authorization with cross-organization restrictions
- Authorization caching and performance testing
- Error handling and fallback policies

To run with a real AuthZEN PDP, specify the --pdp-endpoint flag.
To test fallback behavior, use --fallback-allow.
"
)]
pub struct Args {
    /// Path to SLIM configuration file
    #[arg(
        short,
        long,
        default_value = "config/slim.yml",
        help = "Path to SLIM configuration file"
    )]
    config: PathBuf,

    /// Enable AuthZEN integration
    #[arg(
        long,
        default_value = "true",
        help = "Enable AuthZEN authorization (disable for JWT-only mode)"
    )]
    authzen_enabled: bool,

    /// AuthZEN Policy Decision Point endpoint
    #[arg(
        long,
        default_value = "http://localhost:8080",
        help = "AuthZEN PDP endpoint URL"
    )]
    pdp_endpoint: String,

    /// Enable fallback allow policy
    #[arg(
        long,
        default_value = "true",
        help = "Allow operations when AuthZEN PDP is unavailable (fail-open vs fail-closed)"
    )]
    fallback_allow: bool,

    /// Test fail-closed behavior (deny when PDP unavailable)
    #[arg(
        long,
        help = "Test fail-closed security (equivalent to --fallback-allow=false)"
    )]
    fail_closed: bool,

    /// Demo mode (run through all scenarios)
    #[arg(
        long,
        default_value = "true",
        help = "Run comprehensive demo scenarios"
    )]
    demo_mode: bool,

    /// Verbose authorization logging
    #[arg(
        short,
        long,
        default_value = "false",
        help = "Enable verbose authorization decision logging"
    )]
    verbose: bool,
}

impl Args {
    /// Get configuration file path
    pub fn config(&self) -> &str {
        self.config.to_str().unwrap_or("config/slim.yml")
    }

    /// Check if AuthZEN is enabled
    pub fn authzen_enabled(&self) -> bool {
        self.authzen_enabled
    }

    /// Get PDP endpoint
    pub fn pdp_endpoint(&self) -> String {
        self.pdp_endpoint.clone()
    }

    /// Check if fallback allow is enabled
    pub fn fallback_allow(&self) -> bool {
        // If fail_closed is set, override fallback_allow to false
        if self.fail_closed {
            false
        } else {
            self.fallback_allow
        }
    }

    /// Check if demo mode is enabled
    pub fn demo_mode(&self) -> bool {
        self.demo_mode
    }

    /// Check if verbose logging is enabled
    pub fn verbose(&self) -> bool {
        self.verbose
    }
} 