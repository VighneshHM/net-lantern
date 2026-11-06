mod iface_select;
mod net;
mod netio;
mod platform;
pub mod sim;
mod tcp;
mod types;
#[cfg(windows)]
pub mod win;

pub use iface_select::{choose_interface, neighbor_mac, neighbor_state, AdapterInfo};
pub use net::{ptr_name, RealNetIo};
pub use netio::{HttpResponse, MdnsService, NetIo, NetbiosInfo, SsdpReply};
pub use platform::{
    ArpEntry, Interface, InterfaceKind, NeighborState, PingReply, Platform, PlatformError, TcpProbe,
};
pub use sim::{ScenarioError, SimNetwork};
pub use tcp::tcp_connect_probe;
pub use types::{MacAddr, ParseMacError};
#[cfg(windows)]
pub use win::WindowsPlatform;
