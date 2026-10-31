//! The `netbot` executable's building blocks, split out of `main.rs` so the
//! integration tests can reach them.

pub mod cli;
#[cfg(windows)]
pub mod instance;
