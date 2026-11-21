//! The category Windows gave the connected network, read through the Network
//! List Manager. Read-only: nothing here changes the category.

use windows::core::GUID;
use windows::Win32::Networking::NetworkListManager::{
    INetworkConnection, INetworkListManager, NetworkListManager,
    NLM_NETWORK_CATEGORY_DOMAIN_AUTHENTICATED, NLM_NETWORK_CATEGORY_PRIVATE,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

use super::com::ComScope;
use crate::{NetworkCategory, PlatformError};

/// Map a raw `NLM_NETWORK_CATEGORY` to a [`NetworkCategory`]. An unknown value
/// is treated as Public, the most restrictive.
pub(crate) fn category_from_nlm(raw: i32) -> NetworkCategory {
    match raw {
        r if r == NLM_NETWORK_CATEGORY_PRIVATE.0 => NetworkCategory::Private,
        r if r == NLM_NETWORK_CATEGORY_DOMAIN_AUTHENTICATED.0 => NetworkCategory::Domain,
        _ => NetworkCategory::Public,
    }
}

/// The category to report when no connection belongs to the active adapter:
/// Public if any connected network is Public, else Private. `None` when
/// nothing is connected.
fn fallback_category(categories: &[NetworkCategory]) -> Option<NetworkCategory> {
    if categories.is_empty() {
        None
    } else if categories.contains(&NetworkCategory::Public) {
        Some(NetworkCategory::Public)
    } else {
        Some(NetworkCategory::Private)
    }
}

fn com_err(what: &str) -> impl FnOnce(windows::core::Error) -> PlatformError + '_ {
    move |e| PlatformError::Unavailable {
        reason: format!("could not read the network category ({what}): {e}"),
    }
}

/// The category of the network on the adapter whose GUID is `adapter`, or the
/// fallback over every connected network when none matches.
pub fn read_network_category(adapter: GUID) -> Result<NetworkCategory, PlatformError> {
    let _com = ComScope::enter()?;
    // SAFETY: COM is initialised on this thread for the scope of `_com`, which
    // outlives every interface below.
    let nlm: INetworkListManager =
        unsafe { CoCreateInstance(&NetworkListManager, None, CLSCTX_ALL) }
            .map_err(com_err("open the network list"))?;
    // SAFETY: a getter on a live interface.
    let connections =
        unsafe { nlm.GetNetworkConnections() }.map_err(com_err("list the connections"))?;

    let mut seen = Vec::new();
    loop {
        let mut slot: [Option<INetworkConnection>; 1] = [None];
        let mut fetched = 0u32;
        // SAFETY: one empty slot to fill and a valid out-pointer for the count.
        unsafe { connections.Next(&mut slot, Some(&mut fetched)) }
            .map_err(com_err("walk the connections"))?;
        let Some(connection) = slot[0].take().filter(|_| fetched == 1) else {
            break;
        };
        // SAFETY: getters on a live interface; each returns an owned value.
        let (id, category) = unsafe {
            let id = connection.GetAdapterId().ok();
            let category = connection
                .GetNetwork()
                .and_then(|n| n.GetCategory())
                .map(|c| category_from_nlm(c.0));
            (id, category)
        };
        let Ok(category) = category else { continue };
        if id == Some(adapter) {
            return Ok(category);
        }
        seen.push(category);
    }
    fallback_category(&seen).ok_or(PlatformError::NotConnected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nlm_public_is_public() {
        assert_eq!(category_from_nlm(0), NetworkCategory::Public);
    }

    #[test]
    fn nlm_private_is_private() {
        assert_eq!(category_from_nlm(1), NetworkCategory::Private);
    }

    #[test]
    fn nlm_domain_authenticated_is_domain() {
        assert_eq!(category_from_nlm(2), NetworkCategory::Domain);
    }

    #[test]
    fn fallback_is_public_if_any_is_public_else_private() {
        use NetworkCategory::*;
        assert_eq!(fallback_category(&[Private, Public]), Some(Public));
        assert_eq!(fallback_category(&[Private, Domain]), Some(Private));
        assert_eq!(fallback_category(&[]), None);
    }
}
