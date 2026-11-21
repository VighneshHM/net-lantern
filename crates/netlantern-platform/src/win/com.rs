//! COM initialisation for one blocking call.

use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

use crate::PlatformError;

/// COM initialised on the current thread for as long as this lives. Create it
/// before any COM object so it is dropped after them.
pub(crate) struct ComScope {
    uninitialize: bool,
}

impl ComScope {
    /// Join the multithreaded apartment. A thread already in another apartment
    /// (`RPC_E_CHANGED_MODE`) is used as it is and left initialised.
    pub(crate) fn enter() -> Result<Self, PlatformError> {
        // SAFETY: no reserved pointer; balanced by `CoUninitialize` in `Drop`
        // whenever this call succeeded.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            return Ok(Self {
                uninitialize: false,
            });
        }
        hr.ok().map_err(|e| PlatformError::Unavailable {
            reason: format!("COM could not start: {e}"),
        })?;
        Ok(Self { uninitialize: true })
    }
}

impl Drop for ComScope {
    fn drop(&mut self) {
        if self.uninitialize {
            // SAFETY: pairs the successful `CoInitializeEx` on this thread; every
            // COM object made under this scope has been dropped by now.
            unsafe { CoUninitialize() };
        }
    }
}
