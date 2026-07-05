//! Shared integration-test harness for SLIM black-box tests.
//!
//! Helpers live under [`helpers`]; integration test binaries in `tests/` import this crate.

pub mod binaries;
pub mod constants;
pub mod helpers;

pub use binaries::*;
pub use constants::*;
pub use helpers::*;
