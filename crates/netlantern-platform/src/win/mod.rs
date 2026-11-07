//! Windows readers over IP Helper. This is the only place `unsafe` lives.

mod adapters;
mod arp;
mod icmp;
mod neighbors;
mod platform;

pub use adapters::read_adapters;
pub use neighbors::read_neighbors;
pub use platform::WindowsPlatform;
