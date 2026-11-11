mod iface_select;
mod net;
mod netio;
mod platform;
pub mod sim;
mod tcp;
mod types;
mod wifi;
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
pub use wifi::{
    channel_from_mhz, channel_width_mhz, phy_label, rssi_from_quality, security_label,
    NearbyNetwork, WifiBand, WifiConnection, LOCATION_PERMISSION,
};
#[cfg(windows)]
pub use win::WindowsPlatform;
