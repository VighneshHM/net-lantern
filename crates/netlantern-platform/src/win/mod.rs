//! Windows readers over IP Helper, the WLAN API, the Network List Manager and
//! the firewall policy (read-only COM). This is the only place `unsafe` lives.

mod adapters;
mod arp;
mod com;
mod firewall;
mod icmp;
mod neighbors;
mod netlist;
mod platform;
mod wlan;

pub use adapters::read_adapters;
pub use neighbors::read_neighbors;
pub use platform::WindowsPlatform;
