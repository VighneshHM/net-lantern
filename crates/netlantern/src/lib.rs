//! The `netlantern` executable's building blocks, split out of `main.rs` so the
//! integration tests can reach them.

pub mod cli;
#[cfg(windows)]
pub mod instance;
pub mod notify;
pub mod report;
