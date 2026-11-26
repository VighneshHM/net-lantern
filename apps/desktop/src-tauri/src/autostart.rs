//! Start at login through our own quoted value under `HKCU\...\Run`, which
//! respects Task Manager's switch. A start at login runs with `--hidden`, so
//! it stays in the tray.

use std::io;

use netlantern_platform::startup::{run_command, HkcuRegistry, StartAtLogin};
use netlantern_server::Autostart;

/// The arguments a start at login runs with.
pub const ARGS: &[&str] = &["--hidden"];

/// The host's start-at-login switch. Errors are the OS's free text: the
/// server keeps them out of its answers.
pub struct DesktopAutostart(StartAtLogin<HkcuRegistry>);

impl DesktopAutostart {
    /// The switch for this exe, run with [`ARGS`].
    pub fn for_current_exe() -> io::Result<Self> {
        let exe = std::env::current_exe()?;
        Ok(Self(StartAtLogin::new(
            HkcuRegistry,
            run_command(&exe, ARGS),
        )))
    }

    /// Points an existing start-at-login entry at this exe.
    pub fn repair(&self) -> io::Result<()> {
        self.0.repair()
    }
}

impl Autostart for DesktopAutostart {
    fn is_enabled(&self) -> Result<bool, String> {
        self.0.is_enabled().map_err(|e| e.to_string())
    }

    fn set_enabled(&self, on: bool) -> Result<(), String> {
        self.0.set_enabled(on).map_err(|e| e.to_string())
    }
}
